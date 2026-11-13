# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright>=1.45", "pillow"]
# ///
"""Headless-Chromium smoke test of the site's formula pages, over a built
site (make site):

    uv run tools/site-smoke.py target/site [--shots DIR] [--gallery-tool PATH] [--dist dist --art target/site-art]
        [--base URL]

With --base the pages are fetched from URL (the opendmd-site Worker under
`cf dev`, or a preview deploy) instead of a local server over the folder;
the folder is still read for what the site should hold, so it must be the
build URL serves. There the art index must also carry
`Access-Control-Allow-Origin: *` (target/site/_headers).

- the site's structure: every page reachable from the home page by its
  links, the old addresses (_redirects) leading to pages that exist; the
  menu (the top bar on a laptop, the Menu button on a phone, the guide's and
  the developers' side menus, folded above the page on a phone); the home
  page's buttons, doors and art; on the key pages at laptop and phone
  sizes, light and dark: the theme, no sideways scroll, no CSP violation
  under the page's _headers policy; and none of the words users never see
  (cache, ODM, bucket, manifest, pack, bundle) on the home, install, guide
  and gallery pages;
- the home page's scenes (scene.js, site/theme/SCENES.md): each of them at
  /?scene=NAME, laptop and phone, light and dark: its own picture behind
  the hero and no other scene's fetched, the hero's words legible over it,
  nothing wider than a phone; the name remembered, forgotten by `synth`,
  untouched by a name that isn't one; the address working with no storage;
- the home page's hero show (hero.js): live on its first item, on to the
  second with its caption, no layout shift, no console errors; stopped
  off-screen and by a click; the still kept when less motion is asked for;
- /f/ (a Formula's page, the web app's editor): opened as the Psi
  reference leaves one (`#open`): the name as text (markup stays text), the
  code highlighted, the preview lit and moving, a slider, live edits, none
  of the panel's own buttons, no QR code and no links with code; Copy
  code; Submit to the gallery (GitHub's new-file address, whose file the
  gallery's own checker accepts; a too-long one falls back to copying);
  Open on my panel (asked once: a bad address refused, one on the internet
  asked about; an edited Formula goes to the panel's window it opens, by
  postMessage, to a stand-in there; remembered); a gallery Formula by name
  (`#gallery=<slug>`, to the panel by name too); a choice; nothing to show
  three ways;
- /create/ (Create Art, the web app's Studio), under its _headers CSP with
  no violation: `?q=&platform=` searching Game art at once (the index
  same-origin, nothing asked of opendmd.dev, the pictures stubbed); a
  picture loaded, edited (text, colours) and downloaded, the GIF 128x32;
  GifCities (stubbed) to an animated GIF, trimmed, reversed and
  downloaded with its frames; your own file; none of the words users never
  see; a phone's width; Send to my panel (the address asked once, the
  panel's page opened at #import, the GIF sent to it on its ready with its
  fields, Download after, a blocked pop-up said); `?url=` opening a
  picture, asked directly where the host allows it and through the relay
  (stubbed) where it doesn't, the relay's refusals said in plain words;
  video in: a WebM from a file and an MP4 through the relay as frames on
  the timeline (the reader loaded only then), fitted, trimmed and
  downloaded with the motion kept; the hand-offs: other sites' search as
  links (new tab, "pixel art" and the words), the bookmarklet, the manifest
  and the service worker (scope /create/; a shared GIF and a shared link
  through the share target's POST; nothing else answered by it);
- /studio/ (the Studio, the pixel editor), under its _headers CSP with no
  violation, nothing asked of another site and the GIF writer (pkg-studio)
  fetched only at the first download: a new piece not stored until drawn
  on; its record envelope and its hash (the hash rule worked out again
  here: the same pixels compressed two ways hash the same; Den 16's pinned;
  canon refusing undefined); the envelope's rules (cels per layer, an
  account, a note); a pixel, a line, undo and redo, the eraser, fill and
  picking a colour; the LED view drawn on; renamed and kept over a reload;
  Download GIF a valid 128x32 still of 5000 ms with exactly the colours
  drawn, freezing that rev (a stroke, a download and a stroke at once: one
  draft); a stale rev refused; New piece, opening and deleting drafts; a
  draft that will not read, and localStorage refused; two tabs (a download
  in one does not fork the other; a change in both keeps a copy that says
  where it came from, its name cut to fit, and saving goes on); a newer
  schema's upgrade not blocked by an open tab; on a phone, 8x with the
  two-finger hint, a tap draws, fill then a second finger fills nothing, no
  sideways scroll, both themes;
- /studio/ S2 model (studio-s2-doc): a first visit fetches the seven
  first-paint files, the site's stylesheet, fonts and wordmark and the
  starter pack, nothing else; colour.js and out.js (with pkg-studio) only
  at their triggers; a piece of three shown layers and a hidden one over
  four frames opens as its composite, saves back with the same hash with
  the cels not drawn on as stored, and downloads frame by frame without
  the hidden layer; a whole piece handed in through piece(); an S1
  journal dropped at open; Shade (Light, Half, Heavy anchored to the
  panel, for fills and filled shapes, D cycling); one colour under one key
  after draw, undo, re-pick, redo, draw (and with a paste floating);
  doc.js under 2,000 random operations, and a 4,000-colour paste from one
  key allocator; the hostile-input list (deflate bombs and broken cels,
  records and journals that lie); web-size failing a stray studio/ file;
- /studio/ Import (studio-s2-import): Create Art's converter in a sheet
  over the Studio, a same-origin frame (`/create/?in=studio`): the Studio's
  CSP byte for byte as it was and nothing of the converter fetched without
  Import; a still, an animated GIF, Game art and GifCities (stubbed) and
  videos (any video/*, Start at) into a new piece or a layer, pixel for
  pixel with what Download writes, with `from` and Send's fields; paste
  (exact up to 128x32, else Fit it in Import) and drops routed; the
  cutover's hand-off from /create/ with its switch forced on; the phone's
  full-screen sheet; Import's hostile inputs (forged and lying messages,
  other origins framing /create/, odd videos, bad pastes, frame caps);
- /studio/ tidy (studio-tidy): foreign names, authors, notes, tags and
  from[] as plain text with the joiners kept between characters (files,
  links, Import, and hostile ones by the thousand), Made from quoted, a
  split backup's part opened alone, packs naming absent records, framed
  /create/'s drops; Import in full colour (4,096 and 600 colours pixel for
  pixel, dark colours not turned black), Fit to 256 colours when asked,
  and by itself past a piece's 4,096; from[] entries that aren't text
  refused; blank fillers and flag-smuggled tags dropped;
- /studio/ notices (studio-share): sharing shelved (no Share button, no
  pack's Share link; a #p= that comes in still opens); an import refusal,
  a file that won't open and Send to panel's blocked pop-up each said in
  the window and on top (alert, or status for news), dismissable, at
  laptop and phone sizes, light and dark;
- Open on my panel from every gallery tile (by name; asked once, then one
  press to the panel; Back never hands on again);
- Open on my panel, both ends: an edited Formula sent from /f/ (this build
  as https://opendmd.dev) to the panel's app (this build's web app on
  mock-device.py over target/demo-card, as http://192.168.1.50) is saved
  under Your formulas as it arrives, and is still there, and on the card,
  after a reload;
- the demo's Formulas: no share link, Copy code and Submit;
- /art-index/v1/ (Create Art's game name index, not in the www pack): every
  platform's file published, the committed file byte for byte, served as
  JSON (the CORS header a player's web app needs is the site's _headers',
  which this test's own server cannot show; checked with --base); none left under demo/; the demo's
  Game art searches it same-origin (nothing asked of opendmd.dev; the
  pictures' requests answered locally);
- /formulas/ (the gallery): every example listed with its thumbnail, newest
  first, Open leading to the Formula in the editor; its sort and filter form
  (each sort, Show, Has controls, search, the state in the query and back
  from a link, by keyboard, nothing matching, hover-play after a sort, a
  phone's width, hidden without a script);
- the HTTP API: dev/api.html in the site, linking every frozen copy it
  carries (dev/api/v<version>.html); then a publish-like build of the API
  pages (`site.py --api-only` into a copy of the site's dev/ and styles)
  with an archive standing in for the public repo's dev/api/ (an older
  release's frozen page, kept byte for byte; a stray file and a link, left
  out) and this release frozen: both exist and are listed newest first,
  the frozen page says which release it describes and links the latest;
  with `--api-freeze auto` off a tag, nothing new is frozen (the release
  frozen is `--beta-dist`'s when one is given: the tree's own);
- the changelog (/changelog/): its entries newest first, none newer than
  the newest release the site serves, each linked to its GitHub release,
  the release marked Latest and pre-releases Beta; the same entries in its
  Atom feed; linked from the footer and the install page;
- the install page: the DevKit's button for `--dist`'s release, which
  install/release.json names; install/beta.json the same shape: with
  `--beta-dist`, that beta's (files, manifest and parts under
  install/firmware/beta/) and a second button for it, else release.json
  byte for byte and no beta button; for a release whose dist has the
  offline install (manifest-4mb-offline.json, nvs-offline-4mb.bin), both
  published byte for byte with every part, and its button's box "Play from
  the card only, no WiFi" (a real label, under the site's CSP) switching the
  button's `manifest` attribute to the offline manifest and back, with the
  card file `opendmd.json` (`{"network":"off"}` and a newline) as a download
  beside it; with none, no box;
- /dev/psi/ (the Psi language reference): in the menu, its generated
  tables (each row an id, the name index linking them) and grammar, its
  "On this page", under its own CSP (the docs' and 'wasm-unsafe-eval')
  with no violation; Psi loaded only when an example is near, then every
  live example drawn and lit, Open in editor opening the site's editor
  (/f/#open, the example never in the link); the search and the cheat
  sheet; no sideways scroll on a phone; an old /dev/psi/#E202 sent on to
  the error codes page;
- /dev/psi/errors/ (the error codes): every code an anchor, the common ones
  with a before (highlighted, never drawn) and an after, E407/E408
  internal; the search and the cheat sheet over them;
- an error's code linked to the error codes page: /f/'s editor and the demo's;
- the demo's Settings, against its player in the page (js/demo.js): every
  tab; brightness, the colour, the Formula look, each GIF's stay, the
  playlist, night mode and a rule shown on its panel and in Now's banner;
  panel timing tried and kept, Get beta updates on and off, a new name, a
  key; firmware, WiFi and
  restarting shown with a line saying the demo can't; kept across a reload,
  not into a new session; every tab at laptop and phone sizes (in --shots,
  else target/site-smoke/).

With --shots: the key pages, /f/, the gallery and the 404 page at laptop and
phone sizes, light and dark (whole page, and the first screen as -top), and
the phone's open Menu.
Needs, once, `uv run --with playwright playwright install chromium`.
"""

import argparse
import contextlib
import filecmp
import functools
import http.server
import io
import json
import os
import posixpath
import re
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse
import urllib.request
from pathlib import Path

from PIL import Image
from playwright.sync_api import Error as PlaywrightError, Page, TimeoutError as PlaywrightTimeout, sync_playwright

ROOT = Path(__file__).resolve().parent.parent
SIZES = {"laptop": (1280, 800), "phone": (390, 844)}
failures: list[str] = []


def check(ok: bool, what: str) -> None:
    print(f"  {'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        failures.append(what)


def serve(root: Path) -> str:
    class Quiet(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *a):  # noqa: D401
            pass
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Quiet, directory=str(root)))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return f"http://127.0.0.1:{srv.server_port}"


def new_page(browser, size: str, errors: list[str], scheme: str | None = None) -> Page:
    w, h = SIZES[size]
    phone = size == "phone"
    ctx = browser.new_context(viewport={"width": w, "height": h}, device_scale_factor=3 if phone else 1,
                              is_mobile=phone, has_touch=phone, color_scheme=scheme)
    ctx.grant_permissions(["clipboard-read", "clipboard-write"])
    page = ctx.new_page()
    # (The demo asks /api/status first: a static site answers 404, as opendmd.dev does.)
    # (/f/#gallery=no-such-formula asks for a file that isn't there, on purpose.)
    # (Create Art's relay refusals, the hosts that refuse the browser (stubbed) and a GET of
    # the share target's address (only its POST is the worker's): on purpose.)
    page.on("console", lambda m: m.type == "error" and not any(a in m.location.get("url", "") for a in ("/api/status", "/formulas/src/no-such-formula", "/api/fetch", "nocors.example", "/create/share"))
            and errors.append(f"[{size}] console: {m.text} ({m.location.get('url')})"))
    page.on("pageerror", lambda e: errors.append(f"[{size}] pageerror: {e}"))
    page.on("response", lambda r: r.status >= 400 and "/demo/api/" not in r.url and "/api/status" not in r.url
            and "/formulas/src/no-such-formula" not in r.url and "/api/fetch" not in r.url and "/create/share" not in r.url and errors.append(f"[{size}] HTTP {r.status} {r.url}"))
    return page


def wait_js(page: Page, expr: str, timeout: float = 30000) -> None:
    """page.wait_for_function, but polled with page.evaluate: Playwright's
    wait_for_function evaluates its predicate with eval() in the page, which
    the site's CSP (no 'unsafe-eval', target/site/_headers) refuses under
    --base."""
    end = time.monotonic() + timeout / 1000
    while True:
        try:
            if page.evaluate(expr):
                return
        except PlaywrightError:  # not there yet, or a navigation in between: try again
            if time.monotonic() > end:
                raise
        if time.monotonic() > end:
            raise PlaywrightTimeout(f"wait_js: {expr[:80]} not true after {timeout:.0f} ms")
        page.wait_for_timeout(50)


def lit(page: Page, sel: str) -> int:
    return page.evaluate("""(sel) => { const c = document.querySelector(sel); if (!c || !c.width) return 0;
        const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data; let n = 0;
        for (let i = 0; i < d.length; i += 4) if (d[i] + d[i+1] + d[i+2] > 90) n++; return n; }""", sel)


# /f/ is ready when #f is no longer aria-busy: the name is set before the wasm
# loads, the highlighted code, preview and buttons only after, so waiting for
# #f-name alone raced them (the gallery's "Open leads to" check, flaky under load).
F_READY = "#f:not([hidden]):not([aria-busy])"


def f_ready(page: Page, timeout: float = 20000) -> None:
    """Wait for /f/ to be ready, or to have said why not (no Formula: #f-none,
    #f hidden for good; a load failure: #f-cap), and fail with what it said
    rather than a bare timeout. Arrive from another document (about:blank),
    never /f/ -> /f/#…: f.js reloads on that hashchange, and the old page's
    #f-none would answer."""
    page.wait_for_selector(f"{F_READY}, #f-none:not([hidden])", timeout=timeout)
    if not page.is_visible("#f") or page.is_visible("#f-cap"):
        why = page.evaluate("[...document.querySelectorAll('#f-none, #f-cap')].filter(e => !e.hidden).map(e => e.textContent.trim()).join(' | ')")
        raise AssertionError(f"/f/ ({page.url}) is not showing a Formula: {why}")


def examples(bundled_only: bool = False) -> list[dict]:
    """OpenDMD's examples (opendmd-web/formulas/*.psi): each file's formula
    (without its header), its name (its `name "…"` line), whether it is
    bundled, when it was added (opendmd-web/formulas/added.json), in the
    gallery's order (newest first, then by name)."""
    added = json.loads((ROOT / "opendmd-web/formulas/added.json").read_text())
    out = []
    for p in sorted((ROOT / "opendmd-web/formulas").glob("*.psi")):
        lines = p.read_text(encoding="utf-8").split("\n")
        head = []
        while lines and lines[0].startswith(("-- author:", "-- licence:", "-- license:", "-- bundle")):
            head.append(lines.pop(0))
        src = "\n".join(lines)
        out.append({"slug": p.stem, "src": src, "bundle": "-- bundle" in head, "added": added[p.stem],
                    "name": re.search(r'^name "([^"]*)"', src, re.M).group(1)})
    if bundled_only:  # as formulas.json has them: in file-name order
        return [e for e in out if e["bundle"]]
    by_name = sorted(out, key=lambda e: (e["name"].casefold(), e["slug"]))
    return sorted(by_name, key=lambda e: e["added"], reverse=True)


def gallery_link(base: str, slug: str, **sliders) -> str:
    """A gallery Formula's page, as the gallery's tiles link it (by name)."""
    q = urllib.parse.urlencode({"gallery": slug, **{k: str(v) for k, v in sliders.items()}})
    return f"{base}/f/#{q}"


def open_text(page: Page, base: str, src: str, name: str = "", more: str = "") -> None:
    """/f/ with `src` in it, as the Psi reference's Open in editor leaves it
    (this site's storage, `/f/#open`): the way to any text, as no link
    carries one."""
    page.goto(f"{base}/f/")
    page.evaluate("([s, n]) => localStorage.setItem('odmd.psi.open', JSON.stringify({src: s, name: n}))", [src, name])
    page.goto("about:blank")
    page.goto(f"{base}/f/#open{more}")
    f_ready(page, 20000)


def params(u: str) -> dict:
    return urllib.parse.parse_qs(urllib.parse.urlsplit(u).fragment, keep_blank_values=True)


def frames(page: Page) -> int:
    return int(page.get_attribute("#psi-ed-panel", "data-frames") or 0)


# A stand-in for the panel's web app at 192.168.1.50: says it is ready to
# the page that opened it, and keeps what comes back (in its title).
PANEL_STUB = ("<!doctype html><title>waiting</title><script>addEventListener('message', (e) => { "
              "document.title = JSON.stringify({origin: e.origin, data: e.data}); });"
              "if (opener) opener.postMessage({type: 'opendmd-remix-ready'}, '*');</script>")


def panel_route(ctx, seen: list[str], stub: str = None) -> None:
    ctx.route("http://192.168.1.50/**", lambda r: (seen.append(r.request.url), r.fulfill(body=stub or PANEL_STUB, content_type="text/html")))


# A panel's app from OpenDMD 1.1 on: it says what its compiler knows (`feat`).
PANEL_STUB_11 = PANEL_STUB.replace("{type: 'opendmd-remix-ready'}", "{type: 'opendmd-remix-ready', feat: 1}")


def sent(popup: Page) -> dict:
    """What the stand-in panel got: {origin, data}."""
    popup.wait_for_function("document.title !== 'waiting'", timeout=20000)
    return json.loads(popup.title())


def f_drafts(browser, base: str, errors: list[str], shots: Path | None) -> None:
    """/f/ never loses work: the text as changed is kept in this browser per
    Formula and offered back after a reload (a gallery one and an #open one,
    whose text the reference leaves once); Earlier versions… has it as it
    came, and Go back to this is one undo."""
    e = next(x for x in examples() if not x["bundle"])
    page = new_page(browser, "laptop", errors)
    page.goto(gallery_link(base, e["slug"]))
    f_ready(page)
    orig = page.input_value("#psi-src")
    mine = orig.rstrip("\n") + "\n-- my change"
    page.fill("#psi-src", mine)
    page.wait_for_timeout(600)
    page.goto("about:blank")
    page.goto(gallery_link(base, e["slug"]))
    f_ready(page)
    check(page.is_visible("#f-cont") and "still here" in page.inner_text("#f-cont") and page.input_value("#psi-src") == orig,
          f"/f/: a reload offers the changes back, the gallery's text loaded ({page.inner_text('#f-cont') if page.is_visible('#f-cont') else 'no offer'})")
    if shots:
        page.screenshot(path=str(shots / "f-continue-laptop.png"))
    page.click("#f-cont-go")
    page.wait_for_timeout(400)
    check(page.input_value("#psi-src") == mine and page.locator("#f-cont").count() == 0, "/f/: Continue where you left off puts the changes back")
    page.click("#psi-vers-btn")
    page.wait_for_selector("#psi-vers", timeout=10000)
    rows = page.eval_on_selector_all("#psi-vers button.mi", "els => els.map(e => e.textContent)")
    words = page.inner_text("#psi-vers").lower()
    check(len(rows) == 2 and any("As it came from the gallery" in r for r in rows) and any("Your changes" in r for r in rows)
          and not any(w in words for w in ("cache", "copy", "bucket", "history", "version control")),
          f"/f/: Earlier versions has it as it came and the changes ({rows})")
    k = next(i for i, r in enumerate(rows) if "As it came" in r)
    page.click(f"#psi-v-{k}")
    check(page.is_visible(f"#psi-v-{k}-d .psi-diff .d") and "my change" in page.inner_text(f"#psi-v-{k}-d .psi-diff .d"),
          "/f/: a version shows what going back would take out")
    if shots:
        page.screenshot(path=str(shots / "f-versions-laptop.png"))
    page.click(f"#psi-v-{k}-go")
    page.wait_for_timeout(400)
    back = page.input_value("#psi-src")
    page.keyboard.press("Control+z")
    page.wait_for_timeout(400)
    check(back == orig and page.input_value("#psi-src") == mine, "/f/: Go back to this restores it, and one undo takes that away")
    page.context.close()
    # #open: its text is read once, so a reload starts from the draft's.
    page = new_page(browser, "laptop", errors)
    src = "loop 4\ngray(frac(x + t/4))"
    open_text(page, base, src, "Draft")
    page.fill("#psi-src", src + "\n-- more")
    page.wait_for_timeout(600)
    page.goto("about:blank")
    page.goto(f"{base}/f/#open")
    f_ready(page)
    shown = page.input_value("#psi-src")
    page.click("#f-cont-go")
    page.wait_for_timeout(400)
    check(shown == src and page.input_value("#psi-src") == src + "\n-- more", "/f/#open: a reload still has it, and the changes back")
    # Another #open Formula is its own: no offer, none of the first one's versions.
    other = "loop 2\ngray(y)"
    open_text(page, base, other, "Other")
    page.click("#psi-vers-btn")
    page.wait_for_selector("#psi-vers", timeout=10000)
    rows = page.eval_on_selector_all("#psi-vers button.mi", "els => els.map(e => e.textContent)")
    check(page.locator("#f-cont").count() == 0 and len(rows) == 1 and "As it was opened" in rows[0],
          f"/f/#open: another formula has its own draft and versions ({rows})")
    page.context.close()


def f_page(browser, base: str, errors: list[str], tool: Path | None, shots: Path | None) -> None:
    src = "control size = 0 \"Size\"\nloop 4\nd = len((x - 0.5)*4, y - 0.5)\npal(rainbow, frac(d*(1 + size) - t/2))"
    name = 'Rings <img src=x onerror="window.pwned=1">'
    for size in SIZES:
        page = new_page(browser, size, errors)
        open_text(page, base, src, name)
        wait_js(page, "+(document.querySelector('#psi-ed-panel').dataset.frames || 0) > 5", timeout=20000)
        if size == "laptop":
            check(page.inner_text("#f-name") == name and page.evaluate("!window.pwned && !document.querySelector('#f-name img, #f-ed img')"),
                  "/f/: the name is shown as text")
            check(page.input_value("#psi-src") == src and page.locator("#f-ed .psi-hl .n").count() > 3,
                  "/f/: the web app's editor with the code, highlighted")
            check(page.locator("#f-qr, #f-qr-box, #f-copy-link, #f-demo, #f-add, #psi-back, #psi-save, #psi-ed-gif, #psi-ed-pl, #psi-ed-show").count() == 0,
                  "/f/: no QR code, no links with code, none of the panel's own buttons")
            a0 = lit(page, "#psi-ed-panel")
            f0 = frames(page)
            page.wait_for_timeout(400)
            check(a0 > 1000 and frames(page) > f0, f"/f/: the preview is lit ({a0} px) and moving")
            check(page.inner_text("#psi-ed-sl .psi-sl span") == "Size" and "Loops every 4 s" in page.inner_text("#psi-ed-cap"),
                  "/f/: a slider labelled by its control line, the caption")
            page.click("#f-copy-code")
            check(page.evaluate("navigator.clipboard.readText()") == src, "/f/: Copy code copies the formula")
            # Submit to the gallery: GitHub's new-file page with the file, which the checker accepts.
            page.click("#f-submit")
            page.wait_for_selector("#f-sub:not([hidden]) #s-file", timeout=10000)
            check(page.input_value("#s-file") == "rings-img-src-x-onerror-window-pwned-1",
                  f"/f/: Submit opens its form ({page.input_value('#s-file')})")
            check("name" in page.inner_text("#s-err"), "/f/: the name with < > is refused")
            page.fill("#s-name", "Smoke rings")
            page.fill("#s-desc", "Rings that ripple out")
            page.fill("#s-author", "@smoke")
            page.select_option("#s-lic", "CC-BY-4.0")
            href = page.get_attribute("#s-go", "href")
            q = urllib.parse.parse_qs(urllib.parse.urlsplit(href).query)
            text = q.get("value", [""])[0]
            check(href.startswith("https://github.com/opendmd/opendmd.github.io/new/main/gallery?filename=smoke-rings.psi&value=")
                  and page.inner_text("#s-err") == "" and page.is_hidden("#s-long"), "/f/: Open on GitHub, file name and text in the address")
            shown = page.evaluate("document.querySelector('#s-text').textContent")
            check(text == "-- author: @smoke\n-- licence: CC-BY-4.0\nname \"Smoke rings\"\nabout \"Rings that ripple out\"\n" + src + "\n"
                  and shown == text, f"/f/: the file has the gallery's header and its own name and about lines {text!r}")
            if tool:
                with tempfile.TemporaryDirectory() as d:
                    p = Path(d) / "smoke-rings.psi"
                    p.write_text(text)
                    r = subprocess.run([str(tool), "check", str(p)], capture_output=True, text=True)
                    check(r.returncode == 0, f"/f/: the gallery's checker accepts it: {r.stderr.strip()}")
            if shots:
                page.locator("#f-sub").scroll_into_view_if_needed()
                page.screenshot(path=str(shots / "f-submit-laptop.png"))
            # Live: an edit plays at once, its name line names the page.
            page.fill("#psi-src", 'name "Edited rings"\n' + src.replace("t/2", "t/4"))
            wait_js(page, "document.querySelector('#f-name').textContent === 'Edited rings'", timeout=5000)
            check(page.get_attribute("#psi-ed", "data-err") is None and page.input_value("#psi-name") == "Edited rings",
                  "/f/: edited live, the name follows its name line")
            page.eval_on_selector("#psi-ed-sl-a", "r => { r.value = 0.5; r.dispatchEvent(new Event('input')); }")
            # Open on my panel: asked once, a bad address refused, then remembered;
            # an edited one goes by postMessage to the window it opens.
            seen: list[str] = []
            panel_route(page.context, seen)
            page.click("#f-open")
            check(page.is_visible("#f-ask") and page.input_value("#f-addr") == "opendmd.local", "/f/: Open on my panel asks for the address (opendmd.local)")
            page.fill("#f-addr", "evil.example/x?")
            page.click("#f-go")
            check(page.is_visible("#f-addr-err") and not seen, "/f/: a bad address is refused")
            page.fill("#f-addr", "example.com")
            page.click("#f-go")
            check(page.is_visible("#f-addr-err") and "Press Open again" in page.inner_text("#f-addr-err") and not seen
                  and page.url.startswith(base), "/f/: an address on the internet is asked about first")
            page.fill("#f-addr", "http://192.168.1.50/")
            with page.context.expect_page() as info:
                page.click("#f-go")
            popup = info.value
            got = sent(popup)
            want = {"type": "opendmd-remix", "name": "Edited rings", "src": 'name "Edited rings"\n' + src.replace("t/2", "t/4"), "a": 0.5}
            check(popup.url == "http://192.168.1.50/#remix" and got["origin"] == base and {k: got["data"].get(k) for k in want} == want,
                  f"/f/: an edited one opens the panel's page, nothing in its link, and is sent to it ({popup.url}, {str(got)[:90]})")
            popup.close()
            check(page.inner_text("#f-where-addr") == "192.168.1.50", "/f/: the address is remembered")
            with page.context.expect_page() as info:
                page.click("#f-open")
            check(info.value.url == "http://192.168.1.50/#remix", "/f/: and used straight away the next time")
            info.value.close()
        else:
            check(page.evaluate("document.documentElement.scrollWidth <= innerWidth"), "/f/ (phone): no sideways scroll")
        page.context.close()

    # A gallery Formula as it is: to the panel by its name, a top-level
    # navigation (the slider that moved with it).
    every = examples()
    e = next(x for x in sorted(every, key=lambda x: (x["name"].casefold(), x["slug"])) if not x["bundle"] and "control" in x["src"])
    page = new_page(browser, "laptop", errors)
    seen: list[str] = []
    panel_route(page.context, seen)
    page.goto(f"{base}/f/")
    page.evaluate("localStorage.setItem('odmd.display', '192.168.1.50')")
    page.goto("about:blank")
    page.goto(gallery_link(base, e["slug"]))
    f_ready(page, 20000)
    check(page.input_value("#psi-src").rstrip("\n") == e["src"].rstrip("\n") and page.inner_text("#f-name") == e["name"],
          f"/f/: a gallery Formula by name, its text from the gallery ({e['name']})")
    page.eval_on_selector("#psi-ed-sl-a", "r => { r.value = r.max; r.dispatchEvent(new Event('input')); }")
    page.wait_for_timeout(300)
    with page.expect_navigation(url=re.compile(r"^http://192\.168\.1\.50/")):
        page.click("#f-open")
    q = params(page.url)
    check(q.get("gallery") == [e["slug"]] and "a" in q and "src" not in q and "formula" not in q,
          f"/f/: as it came, it goes to the panel by name ({page.url})")
    page.context.close()

    # A choice (`control shape = word, …`): buttons, the value is the choice's number.
    page = new_page(browser, "laptop", errors)
    seen = []
    panel_route(page.context, seen)
    open_text(page, base, "control shape = dot, ring, bar\ngray(shape/2)", "Picker")
    page.evaluate("localStorage.setItem('odmd.display', '192.168.1.50')")
    page.wait_for_selector("#psi-ed-sl-a[role=radiogroup]", timeout=20000)
    page.click("#psi-ed-sl-a [role=radio] >> text=ring")
    with page.context.expect_page() as info:
        page.click("#f-open")
    got = sent(info.value)
    check(page.inner_text("#psi-ed-sl > div > span") == "shape" and got["data"].get("a") == 1,
          f"/f/: a choice's buttons, ring -> a=1 sent to the panel ({got['data'].get('a')})")
    page.context.close()

    # `phase` or `smooth` to a panel from before 1.1 (its app says no `feat`):
    # not sent, its window closed and the page asks for an update; to a 1.1
    # one, sent. A 1.0 Formula whose own names say phase or smooth goes to
    # either (the site's compiler tells the built-ins from them).
    for stub, src, ok in (
        (PANEL_STUB, "smooth\nloop 2\ngray(frac(x + phase))", False),
        (PANEL_STUB_11, "smooth\nloop 2\ngray(frac(x + phase))", True),
        (PANEL_STUB, 'name "phase smooth"\ncontrol phase 0..1 = 0.5\nfn f(smooth) = smooth*2\ndraw { for k < 4 { dot(k*8, 3) } }\ngray(f(phase)*ink)', True),
        (PANEL_STUB, "draw { for phase < 4 { dot(phase*8, 3) } }\nsmooth = ink\nsmooth", True),
    ):
        page = new_page(browser, "laptop", errors)
        seen = []
        panel_route(page.context, seen, stub)
        open_text(page, base, src, "New words")
        page.evaluate("localStorage.setItem('odmd.display', '192.168.1.50')")
        with page.context.expect_page() as info:
            page.click("#f-open")
        popup = info.value
        if ok:
            got = sent(popup)
            check(got["data"].get("src", "").startswith(src.split("\n")[0]) and page.is_hidden("#f-cap"),
                  f"/f/: sent to the panel ({'1.1' if stub is PANEL_STUB_11 else 'before 1.1'}): {src.splitlines()[0]!r}")
        else:
            page.wait_for_function("!document.querySelector('#f-cap').hidden", timeout=20000)
            for _ in range(50):
                if popup.is_closed():
                    break
                page.wait_for_timeout(100)
            check(popup.is_closed() and "older than OpenDMD 1.1" in page.inner_text("#f-cap"),
                  f"/f/: phase and smooth to an older panel: its window closed, update it ({page.inner_text('#f-cap')!r})")
        page.context.close()

    # A long formula: GitHub can't take it in an address, so the file is copied.
    page = new_page(browser, "laptop", errors)
    long_src = "loop 4\n" + "".join(f"-- {',' * 60}\n" for k in range(50)) + "gray(x)"
    open_text(page, base, long_src, "Long one", "&submit")
    page.wait_for_selector("#f-sub:not([hidden]) #s-desc", timeout=20000)
    page.fill("#s-name", "Long one")
    page.fill("#s-desc", "Long")
    page.fill("#s-author", "me")
    href = page.get_attribute("#s-go", "href")
    check(page.is_visible("#s-long") and href.endswith("/new/main/gallery?filename=long-one.psi"),
          f"/f/: a formula too long for GitHub's address: copy the file instead ({href[:90]}, {page.inner_text('#s-err')})")
    page.click("#s-copy")
    check(page.evaluate("navigator.clipboard.readText()").endswith(long_src + "\n"), "/f/: Copy the file")
    # Nothing named, or a name the gallery doesn't have, or nothing left to open.
    for u, what in ((f"{base}/f/", "no formula"), (f"{base}/f/#gallery=no-such-formula", "a name the gallery doesn't have"),
                    (f"{base}/f/#open", "nothing left to open")):
        page.goto("about:blank")
        page.goto(u)
        page.wait_for_selector("#f-none:not([hidden])", timeout=20000)
        check(page.is_hidden("#f"), f"/f/: {what} says so ({page.inner_text('#f-none-why')[:60]})")
    page.context.close()


# Create Art's page: canned answers for its web sources (no network).
C_INDEX = {"repo": "Stub_-_Repo", "names": ["Other Game (Europe)", "Studio Test (Japan)", "Studio Test (USA)"], "kinds": "123"}
C_GC = [  # GifCities' JSON: a square one and a wide one
    {"url_text": "spinning  mario 2", "weight": 90, "gif": "x/a.gif", "checksum": "SQUARE", "height": 64, "width": 64, "page": "https://web.archive.org/web/2009/a"},
    {"url_text": "cool mario banner 1", "weight": 50, "gif": "x/b.gif", "checksum": "WIDE", "height": 31, "width": 88, "page": "https://web.archive.org/web/2009/b"},
]


def c_fixtures() -> tuple[bytes, bytes]:
    """A 256x64 picture (bands and a box) as PNG, and a 4-frame 88x31 GIF (a box moving right)."""
    from PIL import ImageDraw
    im = Image.new("RGB", (256, 64), (10, 20, 60))
    d = ImageDraw.Draw(im)
    for k in range(8):
        d.rectangle([k * 32, 40, k * 32 + 30, 63], fill=(40 * k % 256, 200 - 20 * k, 90))
    d.rectangle([100, 8, 160, 34], fill=(250, 210, 30))
    png = io.BytesIO()
    im.save(png, "PNG")
    frames = []
    for i in range(4):
        f = Image.new("RGB", (88, 31), (20, 30, 120))
        ImageDraw.Draw(f).rectangle([4 + 18 * i, 6, 24 + 18 * i, 24], fill=(250, 210, 30))
        frames.append(f)
    gif = io.BytesIO()
    frames[0].save(gif, "GIF", save_all=True, append_images=frames[1:], duration=100, loop=0)
    return png.getvalue(), gif.getvalue()


def c_stub(page: Page, base: str, png: bytes, gif: bytes, asked: list[str]) -> None:
    cors = {"access-control-allow-origin": "*"}
    page.route(f"{base}/art-index/v1/SNES.json", lambda r: (asked.append(r.request.url), r.fulfill(status=200, body=json.dumps(C_INDEX), headers={"content-type": "application/json"})))
    page.route("https://raw.githubusercontent.com/**", lambda r: (asked.append(r.request.url), r.fulfill(status=200, body=png, headers={**cors, "content-type": "image/png"})))
    page.route("https://gifcities.archive.org/api/v1/gifsearch*", lambda r: (asked.append(r.request.url), r.fulfill(status=200, body=json.dumps(C_GC), headers={**cors, "content-type": "application/json"})))
    page.route("https://blob.gifcities.org/**", lambda r: (asked.append(r.request.url), r.fulfill(status=200, body=gif, headers={
        "content-type": "image/gif", **(cors if r.request.headers.get("origin") else {})})))
    page.route("https://opendmd.dev/**", lambda r: (asked.append(r.request.url), r.abort()))


def c_data(page: Page) -> dict:
    return page.evaluate("({...document.querySelector('#studio').dataset})")


def c_download(page: Page) -> Image.Image:
    """Save (Download GIF): the file, opened."""
    with page.expect_download(timeout=60000) as dl:
        page.click("#st-save")
    d = dl.value
    im = Image.open(io.BytesIO(Path(d.path()).read_bytes()))
    im.filename_suggested = d.suggested_filename
    return im


# The Studio (/studio/): its canvas is the piece at 1:1 (128 x 32), so a
# pixel is read off it directly.
S_READY = "#studio:not([aria-busy])"
AMBER, NAVY = (247, 165, 49), (22, 29, 82)  # Den 16's keys 9 and 2 (studio/palettes.js)
# Den 16's hash (studio/palettes.js), pinned: s_hash of its body, worked out here.
DEN16 = [("0a0e2a", "Night"), ("161d52", "Navy"), ("2e3a8c", "Indigo"), ("6447c8", "Violet"), ("33d1ff", "Cyan"),
         ("8a4f1d", "Bark"), ("14532d", "Pine"), ("2f9e4f", "Moss"), ("f7a531", "Amber"), ("e8601c", "Ember"),
         ("c92a2a", "Barn"), ("ff4f8b", "Pink"), ("f1ebdd", "Cream"), ("7c7f8a", "Ash"), ("ffffff", "White")]
DEN16_HASH = "sha256-05593132e346e421b3cdad1e1e32b54222bd7c2aba338e2a32cb7507878c2e00"


def s_hash(body) -> str:
    """A record's hash, as store.js hashOf says, written again here (not from
    canon()): every pixel field (a string in a `cels` array, a `px` string)
    as its uncompressed bytes in lowercase hex, then canonical JSON (sorted
    keys, no spaces, UTF-8), SHA-256, 'sha256-' + lowercase hex."""
    import base64
    import hashlib
    import zlib

    def raw(b64: str) -> str:
        return zlib.decompress(base64.b64decode(b64)).hex()

    def form(v):
        if isinstance(v, list):
            return [form(x) for x in v]
        if isinstance(v, dict):
            return {k: ([raw(c) if isinstance(c, str) else form(c) for c in x] if k == "cels" and isinstance(x, list)
                        else raw(x) if k == "px" and isinstance(x, str) else form(x)) for k, x in v.items()}
        return v
    text = json.dumps(form(body), sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return "sha256-" + hashlib.sha256(text.encode()).hexdigest()


def s_page(browser, base: str, site: Path, errors: list[str], size: str = "laptop", scheme: str | None = None, ctx=None,
           init: str = "") -> Page:
    """/studio/ under its _headers CSP, ready; its requests in page.asked."""
    if ctx is None:
        page = new_page(browser, size, errors, scheme=scheme)
    else:
        page = ctx.new_page()
        page.on("pageerror", lambda e: errors.append(f"[studio] pageerror: {e}"))
    page.add_init_script("""document.addEventListener('securitypolicyviolation', e =>
        (window.__csp = window.__csp || []).push(e.violatedDirective + ' ' + e.blockedURI))""" + init)
    page.route(lambda u: urllib.parse.urlsplit(u).path.startswith("/studio/") and urllib.parse.urlsplit(u).path.endswith(("/", ".html")),
               lambda r: (lambda resp: r.fulfill(response=resp, headers={**resp.headers, "content-security-policy": csp_for(site, "/studio/*")}))(r.fetch()))
    page.asked = []
    page.on("request", lambda r: page.asked.append(r.url))
    page.goto(f"{base}/studio/")
    page.wait_for_selector(S_READY, timeout=30000)
    return page


def s_px(page: Page, x: int, y: int) -> tuple:
    return tuple(page.evaluate("([x, y]) => [...document.getElementById('s-canvas').getContext('2d').getImageData(x, y, 1, 1).data.slice(0, 3)]", [x, y]))


def s_lit(page: Page) -> int:
    return page.evaluate("""() => { const d = document.getElementById('s-canvas').getContext('2d').getImageData(0, 0, 128, 32).data;
        let n = 0; for (let i = 0; i < d.length; i += 4) if (d[i] + d[i + 1] + d[i + 2] > 0) n++; return n; }""")


def s_at(page: Page, x: int, y: int) -> tuple[float, float]:
    # The panel in the window (a click further down the page may have scrolled it away).
    page.evaluate("""() => { const r = document.getElementById('s-canvas').getBoundingClientRect();
        if (r.top < 0 || r.bottom > innerHeight) scrollBy(0, r.top - 80); }""")
    b = page.locator("#s-canvas").bounding_box()
    return b["x"] + (x + .5) * b["width"] / 128, b["y"] + (y + .5) * b["height"] / 32


def s_data(page: Page) -> dict:
    return page.evaluate("({...document.getElementById('studio').dataset, saidSaved: document.getElementById('s-saved').textContent})")


def s_kept(page: Page) -> None:
    wait_js(page, "document.getElementById('studio').dataset.saved === 'kept'", timeout=10000)


S_STORE = """async ([what, arg]) => { const m = await import('/studio/store.js'); const s = await m.LocalStore.open();
    try {
    const { items } = await s.list({ type: 'piece' });
    if (what === 'list') return items;
    const id = arg || document.getElementById('studio').dataset.id;
    if (what === 'get') { const r = await s.get(id); return { r, again: r && await m.hashOf(r.body) }; }
    if (what === 'revs') return s.revs(id);
    if (what === 'stale') { const r = await s.get(id); try { await s.put(r, { base: 'stale' }); return 'taken'; } catch (e) { return e.name; } }
    if (what === 'unversioned') { const r = await s.get(id); try { await s.put(r); return 'taken'; } catch (e) { return e.name; } }
    if (what === 'used') return (await s.usedBy(arg)).map((h) => h.id);
    if (what === 'hash') return m.hashOf(arg);
    if (what === 'canon') { try { m.canon(arg === 'undefined' ? { a: undefined } : { a: 1.5 }); return 'taken'; } catch (e) { return 'refused'; } }
    if (what === 'den') { const p = await import('/studio/palettes.js'); await p.ready; return [p.STARTER[0].hash, p.STARTER[0].body]; }
    if (what === 'put') { const { rec, ...o } = arg; try { await s.put({ ...m.record('piece', 'x', {}), ...rec, id: m.newId('piece') }, o); return 'taken'; } catch (e) { return e.message; } }
    if (what === 'cel') return m.packPx(new Uint8Array(4096));
    } finally { s.db.close(); }
}"""


def studio_page(browser, base: str, site: Path, errors: list[str], shots: Path | None) -> None:
    """/studio/ on a laptop, then a phone: draw, keep, download."""
    import base64
    import zlib
    page = s_page(browser, base, site, errors)
    check(s_data(page)["saved"] == "new" and page.evaluate(S_STORE, ["list", None]) == [] and s_lit(page) == 0
          and page.locator("#s-list li").count() == 1,
          f"/studio/: a new piece is empty and not stored until it is drawn on ('{s_data(page)['saidSaved']}')")
    check(s_data(page).get("colour") == "#f7a531", f"/studio/: a new piece's colour is Amber, the bright one ({s_data(page).get('colour')})")
    # Draw: a pixel, a line, undo and redo, the eraser, fill.
    page.click('#s-set .s-sw[data-hex="#f7a531"]')
    page.mouse.click(*s_at(page, 10, 5))
    check(s_px(page, 10, 5) == AMBER and s_lit(page) == 1, f"/studio/: the pencil draws a pixel in the colour picked {s_px(page, 10, 5)}")
    kind = page.inner_text("#s-kind")
    check(kind.endswith("· 1 COLOUR") and page.inner_text("#s-used") == "Colours used: 1"
          and page.evaluate("(() => { const a = document.getElementById('s-name').getBoundingClientRect(), b = document.getElementById('s-kind').getBoundingClientRect(); return b.top >= a.bottom - 2 && Math.abs(b.left - a.left) < 12; })()"),
          f"/studio/: the kind line under the name counts the colours drawn, not off ({kind!r}, {page.inner_text('#s-used')!r})")
    s_kept(page)
    got = page.evaluate(S_STORE, ["get", None])
    r = got["r"] or {}
    envelope = {k: r.get(k) for k in ("type", "rev", "tags", "author", "licence", "from")}
    check(re.fullmatch(r"pc_[0-9a-f]{32}", r.get("id", "")) and envelope == {"type": "piece", "rev": 1, "tags": [], "author": {"name": "", "account": None},
                                                                          "licence": None, "from": []}
          and r["body"]["w"] == 128 and r["body"]["h"] == 32 and [f["ms"] for f in r["body"]["frames"]] == [5000]
          and len(r["body"]["layers"][0]["cels"]) == 1 and r.get("created", "").endswith("Z")
          and r["body"].get("colours") == [{"k": 1, "hex": "#f7a531"}] and "keys" not in r["body"],
          f"/studio/: kept at its first stroke, its record envelope ({r.get('id')}, {envelope}, a still of 5000 ms)")
    check(r.get("hash") == got["again"] == s_hash(r["body"]), f"/studio/: its hash is the hash rule's, worked out here too ({r.get('hash', '')[:20]}…)")
    # The hash is of the pixels, not of how they were compressed: Python's zlib at level 9 and level 1 agree with the page.
    keys = bytes((i * 7) % 16 for i in range(4096))
    bodies = [{"w": 128, "h": 32, "palette": "pl_x", "plays": 0, "frames": [{"ms": 5000}],
               "layers": [{"name": "é", "moves": False, "cels": [base64.b64encode(zlib.compress(keys, lv)).decode()]}]} for lv in (9, 1)]
    hs = [page.evaluate(S_STORE, ["hash", b]) for b in bodies]
    check(hs[0] == hs[1] == s_hash(bodies[0]) and bodies[0] != bodies[1],
          f"/studio/: the same pixels, compressed two ways, hash the same, here and in the page ({hs[0][:20]}…)")
    den_hash, den_body = page.evaluate(S_STORE, ["den", None])
    want = {"colours": [{"k": i + 1, "hex": "#" + h, "name": n} for i, (h, n) in enumerate(DEN16)], "source": "OpenDMD"}
    check(den_hash == DEN16_HASH == s_hash(want) and den_body == want, f"/studio/: Den 16 carries its real hash, pinned ({den_hash[:20]}…)")
    check([page.evaluate(S_STORE, ["canon", w]) for w in ("undefined", "float")] == ["refused", "refused"],
          "/studio/: canon refuses undefined and a number that is not an integer")
    cel = page.evaluate(S_STORE, ["cel", None])
    two = {"w": 128, "h": 32, "palette": "pl_x", "plays": 0, "frames": [{"ms": 100}, {"ms": 100}]}
    puts = [page.evaluate(S_STORE, ["put", {"rec": {"body": {**two, "layers": [{"name": "a", "moves": False, "cels": [cel]},
                                                                            {"name": "b", "moves": True, "cels": [cel, cel]}]}}}]),
            page.evaluate(S_STORE, ["put", {"rec": {"body": {**two, "layers": [{"name": "a", "moves": True, "cels": [cel]}]}}}]),
            page.evaluate(S_STORE, ["put", {"rec": {"author": {"name": "Ada", "account": "acct_1"}, "note": "A one-line note.",
                                                    "body": {**two, "frames": [{"ms": 100}], "layers": [{"name": "a", "moves": False, "cels": [cel]}]}}}]),
            page.evaluate(S_STORE, ["put", {"rec": {"note": 5, "body": {**two, "frames": [{"ms": 100}], "layers": [{"name": "a", "moves": False, "cels": [cel]}]}}}])]
    check(puts[0] == "taken" and "cels" in puts[1] and puts[2] == "taken" and "note" in puts[3],
          f"/studio/: a held layer has one cel, a moving one a cel a frame; an account may be a string; a note is a string {puts}")
    page.evaluate("""async () => { const m = await import('/studio/store.js'); const s = await m.LocalStore.open();
        for (const h of (await s.list({ type: 'piece' })).items) if (h.name === 'x') await s.remove(h.id); s.db.close(); }""")
    check(re.fullmatch(r"pl_0{12}7000\w+", r["body"]["palette"]) is not None and page.evaluate(S_STORE, ["used", r["body"]["palette"]]) == [r["id"]],
          f"/studio/: the piece points at its palette by id, and the store knows it is used there ({r['body']['palette']})")
    page.mouse.move(*s_at(page, 20, 10))
    page.mouse.down()
    page.mouse.move(*s_at(page, 25, 10), steps=3)
    page.mouse.move(*s_at(page, 30, 10), steps=3)
    page.mouse.up()
    row = [s_px(page, x, 10) == AMBER for x in range(20, 31)]
    check(all(row) and s_lit(page) == 12, f"/studio/: a drag draws a line with no gaps ({s_lit(page)} lit)")
    page.click("#s-undo")
    one = s_lit(page)
    page.click("#s-redo")
    check(one == 1 and s_lit(page) == 12, f"/studio/: Undo and Redo ({one}, {s_lit(page)})")
    page.keyboard.press("Control+z")
    one = s_lit(page)
    page.keyboard.press("Control+Shift+z")
    check(one == 1 and s_lit(page) == 12, "/studio/: Ctrl+Z and Ctrl+Shift+Z")
    page.keyboard.press("e")
    page.mouse.click(*s_at(page, 25, 10))
    check(s_lit(page) == 11 and s_px(page, 25, 10) == (0, 0, 0) and page.get_attribute("#s-t-eraser", "aria-pressed") == "true",
          "/studio/: E, the eraser, turns a pixel off")
    page.keyboard.press("g")
    page.click('#s-set .s-sw[data-hex="#161d52"]')
    page.mouse.click(*s_at(page, 100, 20))
    filled = s_lit(page)
    check(filled == 128 * 32 and s_px(page, 25, 10) == NAVY and s_px(page, 10, 5) == AMBER, f"/studio/: G, fill, fills the dark around the drawing ({filled})")
    page.click("#s-undo")
    check(s_lit(page) == 11, "/studio/: fill undone in one step")
    page.mouse.click(*s_at(page, 10, 5), button="right")
    check(s_data(page)["colour"] == "#f7a531", "/studio/: a right-click picks the colour under it")
    # The LED view: the site's .led look; still drawn on.
    page.click("#s-v-led")
    mask = page.evaluate("getComputedStyle(document.getElementById('s-work'), '::after').backgroundImage")
    page.keyboard.press("b")
    page.mouse.click(*s_at(page, 64, 16))
    check("radial-gradient" in mask and s_px(page, 64, 16) == AMBER, "/studio/: LEDS shows the panel's dots and still draws")
    page.click("#s-v-px")
    # Kept over a reload, renamed.
    page.fill("#s-name", "Test card")
    s_kept(page)
    page.reload()
    page.wait_for_selector(S_READY, timeout=30000)
    check(s_px(page, 10, 5) == AMBER and s_px(page, 64, 16) == AMBER and s_lit(page) == 12 and page.input_value("#s-name") == "Test card"
          and page.inner_text("#s-list li.on").startswith("Test card"),
          f"/studio/: the draft is kept over a reload, with its name ({s_lit(page)} lit, {page.input_value('#s-name')!r})")
    check(not any("pkg-studio" in u or "pkg-art" in u for u in page.asked), "/studio/: the GIF writer not fetched before a download")
    # Download GIF: a 128x32 GIF with exactly the colours drawn; that rev frozen; an old error cleared.
    page.evaluate("() => { const e = document.getElementById('s-err'); e.hidden = false; e.textContent = 'an earlier error'; }")
    with page.expect_download(timeout=60000) as dl:
        page.click("#s-gif")
    im = Image.open(io.BytesIO(Path(dl.value.path()).read_bytes()))
    rgbim = im.convert("RGB")
    colours = sorted(c for _, c in rgbim.getcolors())
    check(im.format == "GIF" and im.size == (128, 32) and getattr(im, "n_frames", 1) == 1 and rgbim.getpixel((10, 5)) == AMBER
          and rgbim.getpixel((0, 0)) == (0, 0, 0) and colours == [(0, 0, 0), AMBER] and dl.value.suggested_filename == "Test_card.gif"
          and im.info.get("duration") == 5000,
          f"/studio/: Download GIF -> {dl.value.suggested_filename}, {im.format} {im.size}, {getattr(im, 'n_frames', 1)} frame of "
          f"{im.info.get('duration')} ms, colours {colours}")
    wait_js(page, "document.getElementById('s-err').hidden")
    check(page.is_hidden("#s-err"), "/studio/: a download that works clears the error line")
    s_kept(page)
    revs = page.evaluate(S_STORE, ["revs", None])
    now = page.evaluate(S_STORE, ["get", None])["r"]
    check([v["rev"] for v in revs] == [1] and now["rev"] == 2 and revs[0]["hash"] == now["hash"] and s_data(page)["rev"] == "2",
          f"/studio/: the download froze rev 1, the draft goes on as rev 2 ({[v['rev'] for v in revs]}, {now['rev']})")
    with page.expect_download(timeout=60000):
        page.click("#s-gif")
    check([v["rev"] for v in page.evaluate(S_STORE, ["revs", None])] == [1], "/studio/: downloading again unchanged freezes nothing new")
    # A stroke, a download and a stroke at once (before the autosave): all kept, in one draft, no copy.
    page.mouse.click(*s_at(page, 1, 1))
    with page.expect_download(timeout=60000):
        page.click("#s-gif")
    page.mouse.click(*s_at(page, 2, 1))
    s_kept(page)
    items = page.evaluate(S_STORE, ["list", None])
    now = page.evaluate(S_STORE, ["get", None])["r"]
    check(len(items) == 1 and [v["rev"] for v in page.evaluate(S_STORE, ["revs", None])] == [1, 2] and now["rev"] == 3
          and "copy" not in page.inner_text("#s-saved"),
          f"/studio/: a stroke, a download and a stroke at once: one draft, rev 2 frozen, going on as rev 3 ({[i['name'] for i in items]})")
    check(page.evaluate(S_STORE, ["stale", None]) == "Conflict" and page.evaluate(S_STORE, ["unversioned", None]) == "Conflict",
          "/studio/: the store refuses a write on a stale version, and one that names none when the record exists")
    # Drafts: a new piece, opening the first again, deleting the new one.
    page.click("#s-new")
    wait_js(page, "document.querySelectorAll('#s-list li').length === 2")
    check(s_lit(page) == 0 and page.input_value("#s-name") == "Untitled" and len(page.evaluate(S_STORE, ["list", None])) == 1,
          "/studio/: New piece, an empty draft beside the first, not stored yet")
    page.mouse.click(*s_at(page, 0, 0))
    s_kept(page)
    page.click("#s-list li:not(.on) .s-open")
    wait_js(page, "document.getElementById('s-name').value === 'Test card'")
    check(s_px(page, 10, 5) == AMBER, "/studio/: a draft opens from the list")
    page.once("dialog", lambda d: d.accept())
    page.click("#s-list li:not(.on) .s-del")
    wait_js(page, "document.querySelectorAll('#s-list li').length === 1")
    check(len(page.evaluate(S_STORE, ["list", None])) == 1, "/studio/: a draft deleted (after asking)")
    hit = sorted({m.group(0) for m in HIDDEN_WORDS.finditer(page.inner_text("body"))})
    check(not hit, f"/studio/: none of the words users never see {hit}")
    off = [u for u in page.asked if not u.startswith(base) and not u.startswith(("blob:", "data:"))]
    check(not off and any("pkg-studio" in u for u in page.asked) and not any("pkg-art" in u for u in page.asked),
          f"/studio/: nothing asked of another site, the GIF writer (pkg-studio, not the art module) same-origin {off[:2]}")
    check(not page.evaluate("window.__csp || []"), f"/studio/: no CSP violation {page.evaluate('window.__csp || []')[:2]}")
    if shots:
        page.screenshot(path=str(shots / "studio-laptop.png"), full_page=True)
    # A draft that will not read: said, and the Studio starts on a new piece.
    page.evaluate("""async () => { const m = await import('/studio/store.js'); const s = await m.LocalStore.open();
        const [h] = (await s.list({ type: 'piece' })).items; const r = await s.get(h.id);
        r.body.layers[0].cels[0] = btoa('not a picture');
        await new Promise((ok) => { const t = s.db.transaction('records', 'readwrite'); t.objectStore('records').put({ ...r, seq: 999 }); t.oncomplete = ok; });
        s.db.close(); }""")
    page.reload()
    page.wait_for_selector(S_READY, timeout=30000)
    check(page.is_visible("#s-err") and "can’t be opened: its picture is damaged" in page.inner_text("#s-err") and s_data(page)["saved"] == "new"
          and not re.search(r"header|Error|DOMException|incorrect", page.inner_text("#s-err")),
          f"/studio/: a draft that will not read is said, and a new piece opens ('{page.inner_text('#s-err')[:60]}')")
    page.context.close()
    # A browser that refuses localStorage still starts, draws and keeps the draft.
    page = s_page(browser, base, site, errors, init="""; for (const k of ['getItem', 'setItem']) Storage.prototype[k] = () => { throw new DOMException('refused', 'SecurityError'); };""")
    page.mouse.click(*s_at(page, 3, 3))
    s_kept(page)
    check(s_lit(page) == 1 and page.is_hidden("#s-err"), "/studio/: with localStorage refused, it starts, draws and keeps the draft")
    page.context.close()
    # A phone: opens close up (8x) with the two-finger hint; a tap draws; zoomed, the panel scrolls in its box, never the page.
    for scheme in ("light", "dark"):
        page = s_page(browser, base, site, errors, "phone", scheme)
        z = page.inner_text("#s-z")
        hint = page.is_visible("#s-touch") and "Two fingers" in page.inner_text("#s-touch")
        page.touchscreen.tap(*s_at(page, 5, 16))
        lit = s_lit(page)
        wide = page.evaluate("document.documentElement.scrollWidth - innerWidth")
        page.click("#s-z-in")
        inner = page.evaluate("[document.getElementById('s-work').offsetWidth, document.getElementById('s-scroll').clientWidth]")
        wide2 = page.evaluate("document.documentElement.scrollWidth - innerWidth")
        bg = page.evaluate("getComputedStyle(document.body).backgroundColor")
        lum = sum(int(x) for x in re.findall(r"\d+", bg)[:3])
        check(z == "8×" and hint and lit == 1 and wide <= 0 and wide2 <= 0 and inner[0] > inner[1] and (lum > 600 if scheme == "light" else lum < 120),
              f"/studio/ (phone, {scheme}): opens at {z} with the hint, a tap draws ({lit}), zoomed the panel scrolls in its box {inner}, "
              f"no sideways scroll ({wide}, {wide2} px), the theme ({bg})")
        check(not page.evaluate("window.__csp || []"), f"/studio/ (phone, {scheme}): no CSP violation")
        if scheme == "dark":
            # Fill, then a second finger (a pan): nothing filled, nothing to undo. One finger alone fills.
            page.click("#s-t-fill")
            cdp = page.context.new_cdp_session(page)
            (x1, y1), (x2, y2) = s_at(page, 10, 20), s_at(page, 30, 20)
            pt = lambda x, y, i: {"x": x, "y": y, "id": i}
            cdp.send("Input.dispatchTouchEvent", {"type": "touchStart", "touchPoints": [pt(x1, y1, 1)]})
            cdp.send("Input.dispatchTouchEvent", {"type": "touchStart", "touchPoints": [pt(x1, y1, 1), pt(x2, y2, 2)]})
            cdp.send("Input.dispatchTouchEvent", {"type": "touchMove", "touchPoints": [pt(x1 - 20, y1, 1), pt(x2 - 20, y2, 2)]})
            cdp.send("Input.dispatchTouchEvent", {"type": "touchEnd", "touchPoints": []})
            page.wait_for_timeout(100)
            panned = s_lit(page)
            page.touchscreen.tap(*s_at(page, 10, 20))
            check(panned == 1 and s_lit(page) == 128 * 32,
                  f"/studio/ (phone): fill then a second finger fills nothing ({panned} lit); one finger alone fills ({s_lit(page)})")
        page.context.close()


def studio_tabs(browser, base: str, site: Path, errors: list[str]) -> None:
    """One draft open in two tabs: a download in one (its rev moves on) does
    not fork the other; then the second to change the pixels keeps its work
    as a copy, its `from` naming the draft, even with a name at the limit;
    and a newer Studio's database upgrade is not blocked by an open tab,
    which then says it is not keeping drafts rather than hanging."""
    ctx = browser.new_context(viewport={"width": 1280, "height": 800})
    ctx.on("download", lambda d: None)
    a = s_page(browser, base, site, errors, ctx=ctx)
    a.mouse.click(*s_at(a, 3, 3))
    a.fill("#s-name", "N" * 118)
    s_kept(a)
    b = s_page(browser, base, site, errors, ctx=ctx)
    check(s_data(a)["id"] == s_data(b)["id"], "/studio/ two tabs: both open the same draft")
    a.click("#s-gif")
    wait_js(a, "document.getElementById('studio').dataset.rev === '2'")
    b.mouse.click(*s_at(b, 120, 30))
    s_kept(b)
    items = b.evaluate(S_STORE, ["list", None])
    check(len(items) == 1 and s_data(b)["rev"] == "2" and "copy" not in s_data(b)["saidSaved"],
          f"/studio/ two tabs: a download in one does not fork the other (it goes on at rev {s_data(b)['rev']}, {len(items)} draft)")
    a.wait_for_timeout(1100)  # dates are to the second: the copy's is its own
    a.mouse.click(*s_at(a, 60, 3))
    s_kept(a)
    items = a.evaluate(S_STORE, ["list", None])
    copy = next((i for i in items if i["name"].endswith("(copy)")), {})
    orig = s_data(b)["id"]
    f = (copy.get("from") or [{}])[0]
    first = next((i for i in items if i["id"] == orig), {})
    check(len(items) == 2 and f.get("id") == orig and re.fullmatch(r"sha256-[0-9a-f]{64}", f.get("hash", "")) and len(copy["name"]) <= 120
          and "another tab" in s_data(a)["saidSaved"] and copy.get("created") and copy["created"] != first.get("created"),
          f"/studio/ two tabs: the later change kept as a copy made from the draft (its hash too), made now, the name cut to fit "
          f"({len(copy.get('name', ''))} characters, '{s_data(a)['saidSaved']}')")
    a.mouse.click(*s_at(a, 61, 3))
    s_kept(a)
    check(s_data(a)["saved"] == "kept", "/studio/ two tabs: saving goes on after the copy")
    # A newer schema opened elsewhere: the open tabs let go; their next save says so.
    up = b.evaluate("""() => new Promise((ok) => { const r = indexedDB.open('opendmd-studio', 99);
        r.onsuccess = () => { r.result.close(); ok('opened'); }; r.onblocked = () => ok('blocked'); r.onerror = () => ok('error'); })""")
    a.mouse.click(*s_at(a, 62, 3))
    wait_js(a, "document.getElementById('studio').dataset.saved === 'failed'")
    a.mouse.click(*s_at(a, 63, 3))
    wait_js(a, "document.getElementById('studio').dataset.saved === 'failed'")
    check(up == "opened" and s_data(a)["saidSaved"].startswith("Not kept") and "Reload this one" in s_data(a)["saidSaved"],
          f"/studio/: a newer Studio's upgrade is not blocked by an open tab ({up}), which then says '{s_data(a)['saidSaved'][:50]}…' each time")
    ctx.close()


# --- the Studio, S1: its tools, its colours, its journal, snapshots, touch ---
def py_line(x0: int, y0: int, x1: int, y1: int) -> list[tuple[int, int]]:
    """Bresenham's line, both ends, written again here (studio/tools.js line)."""
    out, dx, dy = [], abs(x1 - x0), -abs(y1 - y0)
    sx, sy, e = (1 if x0 < x1 else -1), (1 if y0 < y1 else -1), abs(x1 - x0) - abs(y1 - y0)
    while True:
        out.append((x0, y0))
        if (x0, y0) == (x1, y1):
            return out
        e2 = 2 * e
        if e2 >= dy:
            e += dy
            x0 += sx
        if e2 <= dx:
            e += dx
            y0 += sy


def py_ellipse(x0: int, y0: int, x1: int, y1: int, filled: bool = False) -> set[tuple[int, int]]:
    """Zingl's plotEllipseRect, written again here from the paper (studio/tools.js ellipse)."""
    pts: set[tuple[int, int]] = set()
    a, b = abs(x1 - x0), abs(y1 - y0)
    b1 = b & 1
    dx, dy = 4 * (1 - a) * b * b, 4 * (b1 + 1) * a * a
    err = dx + dy + b1 * a * a
    if x0 > x1:
        x0, x1 = x1, x1 + a
    y0 = min(y0, y1) + ((b + 1) >> 1)
    y1 = y0 - b1
    a, b1 = 8 * a * a, 8 * b * b
    while True:
        pts |= {(x1, y0), (x0, y0), (x0, y1), (x1, y1)}
        e2 = 2 * err
        if e2 <= dy:
            y0, y1, dy = y0 + 1, y1 - 1, dy + a
            err += dy
        if e2 >= dx or 2 * err > dy:
            x0, x1, dx = x0 + 1, x1 - 1, dx + b1
            err += dx
        if x0 > x1:
            break
    while y0 - y1 <= b:
        pts |= {(x0 - 1, y0), (x1 + 1, y0), (x0 - 1, y1), (x1 + 1, y1)}
        y0, y1 = y0 + 1, y1 - 1
    if filled:
        rows: dict[int, tuple[int, int]] = {}
        for x, y in pts:
            lo, hi = rows.get(y, (x, x))
            rows[y] = (min(lo, x), max(hi, x))
        pts = {(x, y) for y, (lo, hi) in rows.items() for x in range(lo, hi + 1)}
    return pts


def py_rect(x0: int, y0: int, x1: int, y1: int, filled: bool = False) -> set[tuple[int, int]]:
    return {(x, y) for x in range(x0, x1 + 1) for y in range(y0, y1 + 1) if filled or x in (x0, x1) or y in (y0, y1)}


def s_pixels(page: Page) -> list[tuple]:
    d = page.evaluate("[...document.getElementById('s-canvas').getContext('2d').getImageData(0, 0, 128, 32).data]")
    return [tuple(d[i:i + 3]) for i in range(0, len(d), 4)]


def s_on(page: Page) -> set[tuple[int, int]]:
    """The lit pixels, as (x, y)."""
    return {(i % 128, i // 128) for i, c in enumerate(s_pixels(page)) if sum(c)}


def s_drag(page: Page, a: tuple[int, int], b: tuple[int, int], shift: bool = False, release: bool = True) -> None:
    page.mouse.move(*s_at(page, *a))
    page.mouse.down()
    if shift:
        page.keyboard.down("Shift")
    page.mouse.move(*s_at(page, (a[0] + b[0]) // 2, (a[1] + b[1]) // 2), steps=2)
    page.mouse.move(*s_at(page, *b), steps=2)
    if release:
        page.mouse.up()
        if shift:
            page.keyboard.up("Shift")


def s_blur(page: Page) -> None:
    """Keys to the page, not the name field (a new piece's name is selected)."""
    page.evaluate("document.activeElement && document.activeElement.blur()")


def s_new(page: Page) -> None:
    """A new, empty piece (the one before it kept)."""
    n = page.locator("#s-list li").count()
    was = s_data(page).get("id", "")
    if page.is_hidden("#s-new") and page.is_visible("#s-tabs"):
        page.click("#s-tabs [data-tab=piece]")  # a phone: New piece is under the Piece tab
    page.click("#s-new")
    wait_js(page, f"document.querySelectorAll('#s-list li').length === {n + (1 if s_data(page)['saved'] != 'new' else 0)} && "
                  f"document.getElementById('studio').dataset.saved === 'new' && document.getElementById('studio').dataset.id !== '{was}' "
                  "&& !document.getElementById('studio').hasAttribute('aria-busy')")
    s_blur(page)
    page.mouse.move(0, 0)


def s_steps(page: Page) -> str:
    return s_data(page).get("steps", "")


S_TOOLS = """async ([w]) => { const T = await import('/studio/tools.js'); const out = []; const p = (x, y) => out.push([x, y]);
    if (w === 'b1') T.brush(1, p)(5, 5); if (w === 'b2') T.brush(2, p)(5, 5); if (w === 'b3') T.brush(3, p)(5, 5);
    return out; }"""


def studio_tools(browser, base: str, site: Path, errors: list[str]) -> None:
    """Every S1 tool, by its shortcut, against the pixels worked out here."""
    page = s_page(browser, base, site, errors)
    s_blur(page)
    tips = page.eval_on_selector_all(".s-tool", "els => els.map(e => [e.dataset.tool, e.title])")
    keys = {"pencil": "(B)", "eraser": "(E)", "line": "(L)", "rect": "(R)", "ellipse": "(O)", "fill": "(G)", "select": "(M)", "pick": "(I)", "stamp": "(S)"}
    shades = page.eval_on_selector_all("#s-shade [data-shade]", "els => els.map(e => [e.dataset.shade, e.title])")
    check(sorted(t for t, _ in tips) == sorted(keys) and all(keys[t] in title for t, title in tips)
          and all(f"({n})" in page.get_attribute(f'.s-size [data-size="{n}"]', "title") for n in "123")
          and [n for n, _ in shades] == ["solid", "light", "half", "heavy"] and all("(D" in t for _, t in shades),
          f"/studio/ tools: every tool and brush size shows its shortcut in its tooltip ({[t for t, _ in tips]})")
    # Line: Bresenham, the drag's two ends.
    page.keyboard.press("l")
    s_drag(page, (2, 2), (17, 9))
    check(s_on(page) == set(py_line(2, 2, 17, 9)), f"/studio/ tools: L, a line, Bresenham's pixels exactly ({len(s_on(page))})")
    # Shift: flat, upright, or 45 degrees.
    s_new(page)
    page.keyboard.press("l")
    s_drag(page, (20, 20), (32, 23), shift=True)
    flat = s_on(page)
    s_drag(page, (40, 5), (46, 10), shift=True)
    diag = s_on(page) - flat
    s_drag(page, (60, 2), (62, 20), shift=True)
    up = s_on(page) - flat - diag
    check(flat == {(x, 20) for x in range(20, 33)} and diag == {(40 + i, 5 + i) for i in range(7)} and up == {(60, y) for y in range(2, 21)},
          f"/studio/ tools: Shift snaps a line flat, to 45° and upright ({len(flat)}, {len(diag)}, {len(up)})")
    # Rectangles and ellipses, edge and filled.
    s_new(page)
    page.keyboard.press("r")
    s_drag(page, (2, 2), (12, 9))
    page.keyboard.press("Shift+R")
    s_drag(page, (20, 2), (25, 6))
    page.keyboard.press("o")
    s_drag(page, (40, 2), (54, 12))
    page.keyboard.press("Shift+O")
    s_drag(page, (70, 3), (90, 30))
    want = py_rect(2, 2, 12, 9) | py_rect(20, 2, 25, 6, True) | py_ellipse(40, 2, 54, 12) | py_ellipse(70, 3, 90, 30, True)
    got = s_on(page)
    check(got == want, f"/studio/ tools: R and Shift+R, O and Shift+O: rectangles and ellipses, edge and filled, exactly ({len(got)} of {len(want)}, "
                       f"{sorted(got ^ want)[:6]})")
    check(py_ellipse(0, 0, 4, 4) == {(1, 0), (2, 0), (3, 0), (0, 1), (4, 1), (0, 2), (4, 2), (0, 3), (4, 3), (1, 4), (2, 4), (3, 4)}
          and py_ellipse(0, 0, 6, 3) == {(x, 0) for x in range(1, 6)} | {(x, 3) for x in range(1, 6)} | {(0, 1), (6, 1), (0, 2), (6, 2)},
          "/studio/ tools: the ellipse rule worked out here draws a 5x5 circle and a 7x4 ellipse as drawn by hand")
    page.keyboard.press("Shift+R")
    s_drag(page, (100, 20), (110, 25), shift=True)
    sq = s_on(page) - got
    check(sq == py_rect(100, 20, 105, 25, True) or sq == py_rect(100, 20, 110, 30, True),
          f"/studio/ tools: Shift while drawing a rectangle makes it a square ({len(sq)} pixels)")
    # A shape is seen while it is dragged, and kept only when the button lifts.
    s_new(page)
    page.keyboard.press("o")
    before = s_steps(page)
    s_drag(page, (10, 5), (30, 20), release=False)
    seen, mid_steps = s_on(page), s_steps(page)
    page.mouse.up()
    check(seen == py_ellipse(10, 5, 30, 20) and mid_steps == before and s_steps(page) != before,
          f"/studio/ tools: the ellipse is seen while dragged ({len(seen)} lit, steps {before} -> {mid_steps}), a step once let go ({s_steps(page)})")
    # Ctrl+Z is ignored during a stroke.
    page.keyboard.press("b")
    n0 = len(s_on(page))
    page.mouse.move(*s_at(page, 60, 28))
    page.mouse.down()
    page.mouse.move(*s_at(page, 66, 28), steps=3)
    page.keyboard.press("Control+z")
    page.mouse.move(*s_at(page, 70, 28), steps=2)
    page.mouse.up()
    check(len(s_on(page)) == n0 + 11 and (60, 28) in s_on(page), f"/studio/ tools: Ctrl+Z during a stroke is ignored ({len(s_on(page)) - n0} drawn)")
    # Brushes 1, 2 and 3.
    s_new(page)
    page.keyboard.press("2")
    page.mouse.click(*s_at(page, 10, 10))
    two = s_on(page)
    page.keyboard.press("3")
    page.mouse.click(*s_at(page, 30, 10))
    three = s_on(page) - two
    page.keyboard.press("1")
    page.keyboard.press("l")
    page.keyboard.press("3")
    s_drag(page, (50, 10), (60, 10))
    thick = s_on(page) - two - three
    page.keyboard.press("1")
    check(two == {(10, 10), (11, 10), (10, 11), (11, 11)} and three == {(x, y) for x in (29, 30, 31) for y in (9, 10, 11)}
          and thick == {(x, y) for x in range(49, 62) for y in (9, 10, 11)},
          f"/studio/ tools: 2 and 3 make the pencil and the line 2 and 3 pixels wide ({len(two)}, {len(three)}, {len(thick)})")
    # Shade Half (D twice from Solid) on a fill: every other pixel of the region.
    s_new(page)
    page.keyboard.press("g")
    page.keyboard.press("d")
    page.keyboard.press("d")
    page.mouse.click(*s_at(page, 64, 16))
    got = s_on(page)
    check(got == {(x, y) for x in range(128) for y in range(32) if (x + y) % 2 == 0} and s_data(page).get("shade") == "half",
          f"/studio/ tools: G then D, D: a fill at Half shade, a checkerboard ({len(got)} lit)")
    page.keyboard.press("Control+z")
    page.click("#s-shade [data-shade=solid]")
    # The eyedropper: I, then back to the tool before it.
    page.keyboard.press("b")
    page.click('#s-set .s-sw[data-hex="#33d1ff"]')
    page.mouse.click(*s_at(page, 3, 3))
    page.click('#s-set .s-sw[data-hex="#f7a531"]')
    page.keyboard.press("i")
    page.mouse.click(*s_at(page, 3, 3))
    check(s_data(page)["colour"] == "#33d1ff" and s_data(page)["tool"] == "pencil", f"/studio/ tools: I picks the colour under it, then the pencil again ({s_data(page)['colour']}, {s_data(page)['tool']})")
    # The selection: move, flip, copy and paste, delete; each one undo step.
    s_new(page)
    page.keyboard.press("Shift+R")
    s_drag(page, (2, 2), (5, 4))
    page.keyboard.press("b")
    page.mouse.click(*s_at(page, 5, 5))
    shape0 = py_rect(2, 2, 5, 4, True) | {(5, 5)}
    page.keyboard.press("m")
    s_drag(page, (2, 2), (5, 5))
    check(s_data(page)["sel"] == "2,2,4,4" and page.is_visible("#s-sel") and page.is_visible("#s-selbar"), f"/studio/ tools: M selects a box ({s_data(page)['sel']})")
    st = s_steps(page)
    s_drag(page, (3, 3), (13, 3))
    floating = s_on(page)
    page.keyboard.press("Enter")
    moved = {(x + 10, y) for x, y in shape0}
    check(floating == moved and s_on(page) == moved and s_steps(page) != st, f"/studio/ tools: dragged inside, the box's pixels move; Enter puts them down ({len(s_on(page))})")
    page.keyboard.press("Control+z")
    back = s_on(page)
    page.keyboard.press("Control+Shift+z")
    check(back == shape0 and s_on(page) == moved, "/studio/ tools: the move undone and redone in one step")
    s_drag(page, (12, 2), (15, 5))
    page.keyboard.press("Shift+H")
    page.keyboard.press("Enter")
    flipped = {(12 + 15 - x, y) for x, y in moved}
    check(s_on(page) == flipped, f"/studio/ tools: Shift+H flips the box left to right ({len(s_on(page))})")
    s_drag(page, (12, 2), (15, 5))
    page.keyboard.press("Shift+V")
    page.keyboard.press("Enter")
    vflip = {(x, 2 + 5 - y) for x, y in flipped}
    check(s_on(page) == vflip, "/studio/ tools: Shift+V flips it top to bottom")
    s_drag(page, (12, 2), (15, 5))
    page.keyboard.press("Control+c")
    page.keyboard.press("Control+v")
    px0, py0, *_ = (int(v) for v in s_data(page)["sel"].split(",")[:4])
    s_drag(page, (px0 + 1, py0 + 1), (33, 13))
    page.keyboard.press("Enter")
    two = vflip | {(x + 20, y + 10) for x, y in vflip}
    check(s_on(page) == two, f"/studio/ tools: Ctrl+C, Ctrl+V and a drag: a copy beside it ({len(s_on(page))})")
    s_drag(page, (12, 2), (15, 5))
    page.keyboard.press("Delete")
    check(s_on(page) == two - vflip and not s_data(page)["sel"], "/studio/ tools: Delete clears the box")
    page.click("#s-selbar [data-act=paste]")
    at = [int(v) for v in s_data(page)["sel"].split(",")[:4]]
    page.click("#s-selbar [data-act=done]")
    # It lands in the middle of what is in view (the whole panel here): 4 x 4 about (64, 16).
    check(at == [62, 14, 4, 4] and s_on(page) == (two - vflip) | {(x - 12 + 62, y - 2 + 14) for x, y in vflip},
          f"/studio/ tools: the selection bar's Paste lands in the middle of the view, and Done puts it down ({at})")
    # A paste into a piece without its colours: they wait to join it; the paste undone or deleted, they are let go
    # (another colour picked first: the paste's amber is the new piece's picked colour, which stays picked).
    s_new(page)
    page.click('#s-set .s-sw:not([data-hex="#f7a531"])')
    p0 = int(s_data(page)["pending"])
    for let_go in ("Control+z", "Delete"):
        page.keyboard.press("Control+v")
        p1 = int(s_data(page)["pending"])
        page.keyboard.press(let_go)
        p2 = int(s_data(page)["pending"])
        check(p1 > p0 and p2 == p0 and not s_data(page)["sel"] and not s_on(page),
              f"/studio/ tools: a paste's new colours let go when it is {'undone' if let_go == 'Control+z' else 'deleted'} ({p0}, {p1}, {p2})")
    # Undone steps keep their colours' keys: a paste's new colour never takes one, so a step redone while the
    # paste floats keeps its colour when the paste is put down.
    s_new(page)
    page.keyboard.press("b")
    page.keyboard.press("1")
    sws = page.locator('#s-set .s-sw:not([data-hex="#f7a531"])')
    hex_a = sws.nth(0).get_attribute("data-hex")
    sws.nth(0).click()
    page.mouse.click(*s_at(page, 3, 3))
    sws.nth(1).click()
    page.mouse.click(*s_at(page, 5, 3))
    page.keyboard.press("Control+z")
    page.keyboard.press("Control+z")
    page.keyboard.press("Control+v")
    page.keyboard.press("Control+Shift+z")
    page.keyboard.press("Enter")
    got = "#%02x%02x%02x" % s_pixels(page)[3 * 128 + 3]
    check(got == hex_a and s_data(page)["steps"].startswith("2/"),
          f"/studio/ tools: a step redone while a paste floats keeps its colour when the paste is put down ({got}, {hex_a}, {s_data(page)['steps']})")
    check(not page.evaluate("window.__csp || []"), f"/studio/ tools: no CSP violation {page.evaluate('window.__csp || []')[:2]}")
    page.context.close()


S_WIDE = """async ([n]) => { const m = await import('/studio/studio.js');
    const keys = new Uint16Array(4096), colours = [];
    for (let k = 1; k <= n; k++) colours.push({ k, hex: '#' + (k * 4093).toString(16).padStart(6, '0') });
    for (let i = 0; i < 4096; i++) keys[i] = (i % n) + 1;
    await m.piece({ name: 'Wide', keys, colours }); }"""


def studio_colours(browser, base: str, site: Path, errors: list[str], shots: Path | None) -> None:
    """The colour model: any colour joins the piece; swatch sets; palettes of
    one's own and Lospec's; past 256 colours, two-byte keys and a warning."""
    import base64
    import zlib
    page = s_page(browser, base, site, errors)
    s_blur(page)
    wait_js(page, "document.getElementById('studio').dataset.starter !== undefined", timeout=20000)
    opts = page.eval_on_selector_all("#s-pal optgroup[label=Starter] option", "els => els.map(e => e.value)")
    pack = json.loads((site / "studio/starter.dmdpack").read_text())
    root = next(r for r in pack["records"] if r["id"] == pack["root"])
    members = {m["id"] for m in root["body"]["members"]}
    starter_pals = [r["id"] for r in pack["records"] if r["type"] == "palette"]
    check(s_data(page)["starter"] == "16" and sorted(opts) == sorted(starter_pals) and set(opts) <= members
          and not any(re.fullmatch(r"pl_0{12}7000\w+", o) for o in opts),
          f"/studio/ colours: the starter palettes in the menu, by the pack's member ids ({len(opts)})")
    # A colour from another swatch set joins the piece.
    gb = next(r for r in pack["records"] if r["type"] == "palette" and r["name"].startswith("Game Boy DMG"))
    page.select_option("#s-pal", gb["id"])
    page.click(f'#s-set .s-sw[data-hex="{gb["body"]["colours"][0]["hex"]}"]')
    n = page.locator("#s-sw .s-sw").count()
    steps0 = s_steps(page)
    check(s_data(page)["colour"] == gb["body"]["colours"][0]["hex"] and n == 2 and page.locator("#s-sw .s-sw.on.new").get_attribute("aria-label") == gb["body"]["colours"][0]["hex"]
          and steps0 == "0/0" and s_data(page)["saved"] == "new",
          f"/studio/ colours: a swatch from {gb['name']} is picked, shown as not drawn yet: no step, nothing kept ({n - 1} swatch and off, steps {steps0})")
    page.click("#s-pals")
    page.wait_for_selector("#s-palx .s-pill")
    ro = page.inner_text("#s-palx")
    check("read-only" in ro and page.locator("#s-palx [data-a=rename]").count() == 0 and page.locator("#s-palx [data-a=delete]").count() == 0,
          "/studio/ colours: a starter palette is read-only (no Rename, no Delete)")
    page.click("#s-pals")
    # The picker: any colour, by hex; it joins the piece.
    page.click("#s-add")
    page.wait_for_selector("#s-picker .s-hex")
    page.fill("#s-picker .s-hex", "#123456")
    page.click("#s-picker [data-a=add]")
    page.keyboard.press("b")
    page.mouse.click(*s_at(page, 7, 7))
    s_kept(page)
    r = page.evaluate(S_STORE, ["get", None])["r"]
    check(s_data(page)["colour"] == "#123456" and s_px(page, 7, 7) == (0x12, 0x34, 0x56) and r["body"]["colours"] == [{"k": 1, "hex": "#123456"}]
          and r["body"]["palette"] == gb["id"] and s_steps(page) == "1/1",
          f"/studio/ colours: the picker's #123456 joins the piece's colours in the stroke that draws with it, one step ({r['body']['colours']}, {s_steps(page)})")
    page.mouse.move(*s_at(page, 64, 16))
    page.click("#s-picker .s-sv")
    hexv = page.input_value("#s-picker .s-hex")
    check(re.fullmatch(r"#[0-9a-f]{6}", hexv) is not None and page.get_attribute("#s-picker", "data-colour") == hexv,
          f"/studio/ colours: the saturation and value square sets a colour ({hexv})")
    page.click("#s-add")
    # Remove unused: only what is drawn stays; undone in one step. Then a colour added, and two undos.
    labels = lambda: page.eval_on_selector_all("#s-sw .s-sw", "els => els.map(e => e.getAttribute('aria-label'))")
    page.select_option("#s-pal", "pl_00000000000070008000000000000001")
    page.click('#s-set .s-sw[data-hex="#33d1ff"]')
    page.mouse.click(*s_at(page, 9, 9))
    page.keyboard.press("e")
    page.mouse.click(*s_at(page, 9, 9))
    page.keyboard.press("b")
    two = labels()
    page.click("#s-unused")
    after = labels()
    page.keyboard.press("Control+z")
    back = labels()
    page.click("#s-unused")
    page.click('#s-set .s-sw[data-hex="#f7a531"]')
    page.mouse.click(*s_at(page, 8, 8))
    added = [l for l in labels() if l not in ("Off", "#123456")]
    page.keyboard.press("Control+z")
    page.keyboard.press("Control+z")
    check(two == ["Off", "#123456", "#33d1ff"] and after == ["Off", "#123456"] and back == two and added == ["#f7a531"]
          and labels()[:3] == two and s_px(page, 8, 8) == (0, 0, 0) and s_px(page, 7, 7) == (0x12, 0x34, 0x56),
          f"/studio/ colours: Remove unused keeps what is drawn and is undone; removed, a colour added, two undos: as it was ({after}, {labels()})")
    # The piece's colours as a palette of one's own; renamed; deleted.
    page.click("#s-pals")
    page.wait_for_selector("#s-palx [data-a=save]")
    page.click("#s-palx [data-a=save]")
    wait_js(page, "document.querySelector('#s-pal optgroup[label=Yours] option') !== null")
    pid = page.input_value("#s-pal")
    page.fill('#s-palx input[aria-label="Palette name"]', "My greens")
    page.click("#s-palx [data-a=rename]")
    wait_js(page, "document.querySelector('#s-pal option:checked').textContent === 'My greens'")
    stored = page.evaluate("""async ([id]) => { const m = await import('/studio/store.js'); const s = await m.LocalStore.open();
        const r = await s.get(id); s.db.close(); return r; }""", [pid])
    check(stored and stored["name"] == "My greens" and len(stored["body"]["colours"]) == 2 and stored["from"][0]["id"] == r["id"]
          and re.fullmatch(r"sha256-[0-9a-f]{64}", stored["from"][0]["hash"]) is not None,
          f"/studio/ colours: the piece's colours saved as a palette, made from the piece, renamed ({stored and stored['name']})")
    page.once("dialog", lambda d: d.accept())
    page.click("#s-palx [data-a=delete]")
    wait_js(page, "document.querySelector('#s-pal optgroup[label=Yours]') === null")
    check(page.input_value("#s-pal").startswith("pl_000000000000"), "/studio/ colours: a palette of one's own deleted; the swatches go back to Den 16")
    # Lospec: pasted hex lines, and a .gpl file; nothing fetched.
    page.fill("#s-palx .s-paste", "#ff0000\nff8800\n00ff00\nnot a colour\nff0000")
    page.fill("#s-palx input[placeholder=Name]", "Pasted")
    page.click("#s-palx [data-a=import]")
    wait_js(page, "document.querySelector('#s-pal option:checked').textContent === 'Pasted'")
    hexes = page.eval_on_selector_all("#s-set .s-sw", "els => els.map(e => e.dataset.hex)")
    gpl = tempfile.NamedTemporaryFile("w", suffix=".gpl", delete=False)
    gpl.write("GIMP Palette\nName: Sunset Four\nColumns: 4\n# a comment\n255 136  0\tOrange\n 34  17 51 Plum\n0 0 0\n255 255 255\n")
    gpl.close()
    page.set_input_files("#s-palx input[type=file]", gpl.name)
    wait_js(page, "document.querySelector('#s-palx .s-paste').value.length > 0")
    page.fill("#s-palx input[placeholder=Name]", "")
    page.click("#s-palx [data-a=import]")
    wait_js(page, "document.querySelector('#s-pal option:checked').textContent === 'Sunset Four'")
    hexes2 = page.eval_on_selector_all("#s-set .s-sw", "els => els.map(e => e.dataset.hex)")
    os.unlink(gpl.name)
    check(hexes == ["#ff0000", "#ff8800", "#00ff00"] and hexes2 == ["#ff8800", "#221133", "#000000", "#ffffff"],
          f"/studio/ colours: Lospec hex lines pasted, and a .gpl file, make palettes ({hexes}, {hexes2})")
    # Past 256 colours: two bytes a pixel, the warning, the hash, the GIF exact.
    page.evaluate(S_WIDE, [300])
    wait_js(page, "document.getElementById('s-name').value === 'Wide'")
    warn = page.is_visible("#s-many")
    page.keyboard.press("b")
    page.click('.s-sw[data-k="1"]')
    page.mouse.click(*s_at(page, 1, 0))
    s_kept(page)
    r = page.evaluate(S_STORE, ["get", None])["r"]
    want = [(i % 300) + 1 for i in range(4096)]
    want[1] = 1
    raw = b"".join(k.to_bytes(2, "big") for k in want)
    mine = {**r["body"], "layers": [{**r["body"]["layers"][0], "cels": [base64.b64encode(zlib.compress(raw, 6)).decode()]}]}
    check(warn and s_data(page)["table"] == "300" and "slower way" in page.inner_text("#s-many") and r["body"].get("keys") == 2
          and zlib.decompress(base64.b64decode(r["body"]["layers"][0]["cels"][0])) == raw and len(r["body"]["colours"]) == 300
          and r["hash"] == s_hash(mine) == s_hash(r["body"]),
          f"/studio/ colours: past 255, two bytes a pixel (high first), its hash worked out here from the pixels alone, "
          f"and the warning past 256 ({s_data(page)['table']} colours, keys {r['body'].get('keys')})")
    colour = {c["k"]: tuple(bytes.fromhex(c["hex"][1:])) for c in r["body"]["colours"]}
    with page.expect_download(timeout=60000) as dl:
        page.click("#s-gif")
    im = Image.open(io.BytesIO(Path(dl.value.path()).read_bytes()))
    frames, total = 0, 0
    try:
        while True:
            total += im.info.get("duration", 0)
            frames += 1
            im.seek(im.tell() + 1)
    except EOFError:
        pass
    shown = list(im.convert("RGB").getdata())
    check(shown == [colour[k] for k in want] and frames > 1 and total == 5000,
          f"/studio/ colours: its GIF keeps all 300 colours exactly: the whole picture, then {frames - 1} correction(s), {total} ms in all")
    page.reload()
    page.wait_for_selector(S_READY, timeout=30000)
    check(s_pixels(page) == [colour[k] for k in want], "/studio/ colours: a two-byte piece opens again as drawn")
    # A one-byte draft written before S1 (no colours of its own, keys 9) still opens, in its palette's colours.
    cel = base64.b64encode(zlib.compress(bytes([9]) * 4096)).decode()
    old = {"w": 128, "h": 32, "palette": "pl_00000000000070008000000000000001", "plays": 0, "frames": [{"ms": 5000}],
           "layers": [{"name": "Drawing", "moves": False, "cels": [cel]}]}
    check(page.evaluate(S_STORE, ["put", {"rec": {"name": "Old draft", "body": old}}]) == "taken", "/studio/ colours: an S0 draft is still a valid record")
    page.reload()
    page.wait_for_selector(S_READY, timeout=30000)
    page.click("#s-list li:has-text('Old draft') .s-open")
    wait_js(page, "document.getElementById('s-name').value === 'Old draft'")
    check(set(s_pixels(page)) == {AMBER}, "/studio/ colours: an S0 draft opens in its palette's colours")
    check(not page.evaluate("window.__csp || []"), f"/studio/ colours: no CSP violation {page.evaluate('window.__csp || []')[:2]}")
    off = [u for u in page.asked if not u.startswith(base) and not u.startswith(("blob:", "data:"))]
    check(not off, f"/studio/ colours: nothing asked of another site {off[:2]}")
    page.context.close()


def studio_journal(browser, base: str, site: Path, errors: list[str]) -> None:
    """Undo across a reload, 500 steps; snapshots; the last save as the tab hides; persistence said."""
    page = s_page(browser, base, site, errors)
    for x in (1, 2, 3):
        page.mouse.click(*s_at(page, x, 1))
    s_kept(page)
    check(s_data(page).get("persist") in ("yes", "no") and s_data(page)["saidSaved"] == "Kept in this browser"
          and page.is_visible("#s-loose") == (s_data(page)["persist"] == "no"),
          f"/studio/ journal: after the first saved stroke, it asks to be kept for good and says how it stands ({s_data(page).get('persist')}, '{s_data(page)['saidSaved']}')")
    page.keyboard.press("Control+z")
    page.wait_for_timeout(100)
    page.reload()  # before the autosave (at 400 ms): the journal knows the undo
    page.wait_for_selector(S_READY, timeout=30000)
    a = s_on(page)
    st = s_steps(page)
    page.keyboard.press("Control+Shift+z")
    b = s_on(page)
    page.keyboard.press("Control+z")
    page.keyboard.press("Control+z")
    c = s_on(page)
    check(a == {(1, 1), (2, 1)} and st == "2/3" and b == {(1, 1), (2, 1), (3, 1)} and c == {(1, 1)},
          f"/studio/ journal: undo and redo survive a reload ({sorted(a)}, steps {st})")
    # A step the record never saw (the tab closed first): replayed from the journal, and saved.
    # (Journal v2: a pixel step names its cel by layer and frame key.)
    s_kept(page)
    page.evaluate("""async () => { const m = await import('/studio/store.js'); const s = await m.LocalStore.open();
        const id = document.getElementById('studio').dataset.id, j = await s.steps(id), n = j.cur + 1, r = await s.get(id);
        await s.jot(id, { step: { n, l: r.body.layers[0].key, f: r.body.frames[0].key, i: Int32Array.of(5), b: Uint16Array.of(0), a: Uint16Array.of(1) }, cur: n });
        s.db.close(); }""")
    page.reload()
    page.wait_for_selector(S_READY, timeout=30000)
    s_kept(page)
    replayed = s_on(page)
    import base64
    import zlib
    r = page.evaluate(S_STORE, ["get", None])["r"]
    px = zlib.decompress(base64.b64decode(r["body"]["layers"][0]["cels"][0]))
    check(replayed == {(1, 1), (5, 0)} and s_px(page, 5, 0) == AMBER and px[5] == 1 and s_steps(page) == "2/2",
          f"/studio/ journal: a step the record never saw is replayed at the reload, then saved ({sorted(replayed)}, steps {s_steps(page)})")
    page.keyboard.press("Control+z")
    # 500 steps at most.
    for i in range(505):
        page.mouse.click(*s_at(page, i % 128, 4 + i // 128))
    s_kept(page)
    page.reload()
    page.wait_for_selector(S_READY, timeout=30000)
    check(s_steps(page) == "500/500" and len(s_on(page)) == 506, f"/studio/ journal: 500 steps kept, over a reload ({s_steps(page)})")
    # The last save as the tab is hidden: at once, not after the wait.
    page.mouse.click(*s_at(page, 100, 30))
    page.evaluate("Object.defineProperty(document, 'visibilityState', { value: 'hidden', configurable: true }); document.dispatchEvent(new Event('visibilitychange'))")
    page.wait_for_timeout(150)
    r = page.evaluate(S_STORE, ["get", None])["r"]
    px = zlib.decompress(base64.b64decode(r["body"]["layers"][0]["cels"][0]))
    check(px[30 * 128 + 100] == 1, "/studio/ journal: hidden, the tab saves at once")
    page.evaluate("delete document.visibilityState")
    # Snapshots: taken by name, listed, restored (what was there kept first), the restore undone.
    s_new(page)
    page.mouse.click(*s_at(page, 0, 0))
    page.fill("#s-snapname", "First")
    page.click("#s-snap")
    wait_js(page, "document.querySelector('#s-snaps li') !== null")
    page.mouse.click(*s_at(page, 1, 0))
    s_kept(page)
    page.click("#s-snaps li:has-text('First') [data-rev]")
    wait_js(page, "document.querySelectorAll('#s-snaps li').length === 2")
    restored = s_on(page)
    labels = page.eval_on_selector_all("#s-snaps li b", "els => els.map(e => e.textContent)")
    page.keyboard.press("Control+z")
    check(restored == {(0, 0)} and labels == ["Before restoring First", "First"] and s_on(page) == {(0, 0), (1, 0)},
          f"/studio/ journal: a snapshot taken, restored (what was there kept first {labels}), and the restore undone")
    page.reload()
    page.wait_for_selector(S_READY, timeout=30000)
    check(page.eval_on_selector_all("#s-snaps li b", "els => els.map(e => e.textContent)") == ["Before restoring First", "First"],
          "/studio/ journal: snapshots listed after a reload")
    r = page.evaluate(S_STORE, ["get", None])["r"]
    keys = [r["body"]["frames"][0].get("key"), r["body"]["layers"][0].get("key")]
    page.mouse.click(*s_at(page, 5, 5))
    s_kept(page)
    r2 = page.evaluate(S_STORE, ["get", None])["r"]
    check(all(re.fullmatch(r"[0-9a-f]{8}", k or "") for k in keys) and keys == [r2["body"]["frames"][0].get("key"), r2["body"]["layers"][0].get("key")],
          f"/studio/ journal: frames and layers carry stable keys ({keys})")
    check(int(s_data(page).get("drawn", "0")) >= 1, "/studio/ journal: after a save only the open draft is drawn again")
    page.context.close()


def studio_rename_tabs(browser, base: str, site: Path, errors: list[str]) -> None:
    """A rename in one tab is not lost to a stroke in another (the store's version)."""
    ctx = browser.new_context(viewport={"width": 1280, "height": 800})
    a = s_page(browser, base, site, errors, ctx=ctx)
    a.mouse.click(*s_at(a, 3, 3))
    s_kept(a)
    b = s_page(browser, base, site, errors, ctx=ctx)
    a.fill("#s-name", "Renamed in A")
    s_kept(a)
    b.mouse.click(*s_at(b, 9, 9))
    s_kept(b)
    r = b.evaluate(S_STORE, ["get", None])["r"]
    items = b.evaluate(S_STORE, ["list", None])
    check(len(items) == 1 and r["name"] == "Renamed in A" and b.input_value("#s-name") == "Renamed in A" and s_data(a)["journal"] != s_data(b)["journal"],
          f"/studio/ two tabs: a rename in one survives a stroke in the other ('{r['name']}', {len(items)} draft; the journal kept by one tab)")
    ctx.close()


S_PIECE = """async ([n, off]) => { const m = await import('/studio/studio.js');
    const keys = new Uint16Array(4096), colours = [];
    for (let k = 1; k <= n; k++) colours.push({ k, hex: '#' + (k * 4093).toString(16).padStart(6, '0') });
    for (let i = 0; i < 4096; i++) keys[i] = i < off ? 0 : ((i - off) % n) + 1;
    await m.piece({ name: `${n} colours`, keys, colours }); }"""


def studio_edges(browser, base: str, site: Path, errors: list[str]) -> None:
    """The edges: the panel's right edge, 255/256 keys, 256/257 colours, a
    float with holes, another tab's pixels under a journal."""
    import base64
    import zlib
    page = s_page(browser, base, site, errors)
    s_blur(page)
    # A fill reaching x = 127; a selection put down against the right edge.
    page.keyboard.press("l")
    s_drag(page, (120, 0), (120, 31))
    page.keyboard.press("g")
    page.mouse.click(*s_at(page, 125, 5))
    check(s_on(page) == {(x, y) for x in range(120, 128) for y in range(32)}, "/studio/ edges: a fill reaches the right edge, and no further")
    s_new(page)
    page.keyboard.press("Shift+R")
    s_drag(page, (118, 4), (122, 6))
    page.keyboard.press("m")
    s_drag(page, (118, 4), (122, 6))
    s_drag(page, (119, 5), (126, 5))
    page.keyboard.press("Enter")
    check(s_on(page) == {(x, y) for x in range(125, 128) for y in range(4, 7)}, f"/studio/ edges: a selection put down past the right edge is cut there ({sorted(s_on(page))[:4]})")
    # A float with holes put down over a drawing: its off pixels let it show through.
    s_new(page)
    page.click('#s-set .s-sw[data-hex="#161d52"]')
    page.keyboard.press("Shift+R")
    s_drag(page, (40, 0), (60, 20))
    page.click('#s-set .s-sw[data-hex="#f7a531"]')
    page.keyboard.press("r")
    s_drag(page, (2, 2), (6, 6))
    page.keyboard.press("m")
    s_drag(page, (2, 2), (6, 6))
    s_drag(page, (3, 3), (46, 6))
    page.keyboard.press("Enter")
    px = s_pixels(page)
    ring = py_rect(45, 5, 49, 9)
    check(all(px[y * 128 + x] == AMBER for x, y in ring) and all(px[y * 128 + x] == NAVY for x in range(46, 49) for y in range(6, 9))
          and not any(sum(px[y * 128 + x]) for x in range(2, 7) for y in range(2, 7)),
          "/studio/ edges: a box with holes put down over a drawing: the drawing shows through the holes")
    # Keys: 255 colours stay one byte a pixel; the 256th makes two.
    page.evaluate(S_PIECE, [255, 0])
    wait_js(page, "document.getElementById('s-name').value === '255 colours'")
    page.keyboard.press("b")
    page.click('#s-sw .s-sw[data-k="3"]')
    page.mouse.click(*s_at(page, 0, 0))
    s_kept(page)
    one = page.evaluate(S_STORE, ["get", None])["r"]["body"]
    page.click('#s-set .s-sw[data-hex="#f7a531"]')
    page.mouse.click(*s_at(page, 1, 0))
    s_kept(page)
    two = page.evaluate(S_STORE, ["get", None])["r"]["body"]
    raw = zlib.decompress(base64.b64decode(two["layers"][0]["cels"][0]))
    check("keys" not in one and len(zlib.decompress(base64.b64decode(one["layers"][0]["cels"][0]))) == 4096 and two.get("keys") == 2
          and len(raw) == 8192 and raw[2:4] == (256).to_bytes(2, "big") and {"k": 256, "hex": "#f7a531"} in two["colours"],
          f"/studio/ edges: 255 colours, one byte a pixel; the 256th (key 256), two ({one.get('keys')}, {two.get('keys')})")
    # 256 colours: no warning, one table; 257: the warning, the exact GIF, and the fitted one.
    results = {}
    for n in (256, 257):
        page.evaluate(S_PIECE, [n, 0])
        wait_js(page, f"document.getElementById('s-name').value === '{n} colours'")
        page.keyboard.press("b")
        page.click('#s-sw .s-sw[data-k="2"]')
        page.mouse.click(*s_at(page, 0, 0))
        s_kept(page)
        warn = page.is_visible("#s-many")
        with page.expect_download(timeout=60000) as dl:
            page.click("#s-gif")
        im = Image.open(io.BytesIO(Path(dl.value.path()).read_bytes()))
        results[n] = (warn, getattr(im, "n_frames", 1), s_data(page)["table"])
        if n == 257:
            rgb_want = s_pixels(page)
            frames = []
            try:
                while True:
                    frames.append(list(im.convert("RGB").getdata()))
                    im.seek(im.tell() + 1)
            except EOFError:
                pass
            with page.expect_download(timeout=60000) as dl:
                page.click("#s-fit")
            fim = Image.open(io.BytesIO(Path(dl.value.path()).read_bytes()))
            fitted = (getattr(fim, "n_frames", 1), len(fim.convert("RGB").getcolors(4096)), dl.value.suggested_filename)
    check(results[256] == (False, 1, "256") and results[257][0] and results[257][1] == 2 and results[257][2] == "257",
          f"/studio/ edges: 256 colours, no warning and one GIF frame; 257, the warning and a second, exact one {results}")
    check(not any(sum(c) == 0 for c in frames[0]) and frames[-1] == rgb_want,
          "/studio/ edges: past 256, the GIF's first frame is the whole picture (nothing black), its last exact")
    check(fitted[0] == 1 and fitted[1] == 256 and fitted[2] == "257_colours_256.gif",
          f"/studio/ edges: Download fitted to 256 colours: one frame of 256 colours {fitted}")
    # Edge or filled without Shift (for a phone): the shape's segment control.
    page.keyboard.press("r")
    vis = page.is_visible("#s-fillmode")
    page.click("#s-fillmode [data-fill='1']")
    filled = s_data(page)["tool"]
    page.keyboard.press("b")
    check(vis and filled == "rect-filled" and page.is_hidden("#s-fillmode"), f"/studio/ edges: Edge and Filled beside the brushes for a shape ({filled})")
    page.context.close()
    # Another tab changes the pixels; the tab keeping the journal reloads: the journal is let go, the pixels are the other tab's.
    ctx = browser.new_context(viewport={"width": 1280, "height": 800})
    a = s_page(browser, base, site, errors, ctx=ctx)
    for x in (1, 2):
        a.mouse.click(*s_at(a, x, 1))
    s_kept(a)
    b = s_page(browser, base, site, errors, ctx=ctx)
    b.mouse.click(*s_at(b, 9, 9))
    s_kept(b)
    a.reload()
    a.wait_for_selector(S_READY, timeout=30000)
    check(s_on(a) == {(1, 1), (2, 1), (9, 9)} and s_steps(a) == "0/0" and s_data(a)["journal"] == "yes",
          f"/studio/ edges: pixels changed in another tab, the journaled tab reloads: its journal let go, the pixels as stored ({s_steps(a)})")
    ctx.close()


def studio_phone(browser, base: str, site: Path, errors: list[str]) -> None:
    """On a phone: pinch zooms on the same two fingers; the view switch and
    X/Y stay in sight zoomed; a shape cancelled by a second finger; the
    name not selected on a new piece."""
    page = s_page(browser, base, site, errors, "phone", "dark")
    cdp = page.context.new_cdp_session(page)
    pt = lambda x, y, i: {"x": x, "y": y, "id": i}
    vis = page.evaluate("""() => ['s-v-led', 's-xy'].every((id) => { const r = document.getElementById(id).getBoundingClientRect();
        return r.width > 0 && r.left >= 0 && r.right <= innerWidth; })""")
    check(vis and page.inner_text("#s-z") == "8×", "/studio/ (phone): zoomed, PIXELS/LEDS and X/Y stay in sight")
    s_new(page)
    sel = page.evaluate("(() => { const e = document.getElementById('s-name'); return document.activeElement === e && e.selectionEnd > e.selectionStart; })()")
    check(not sel, "/studio/ (phone): a new piece's name is not selected (no keyboard over the panel)")
    # A line, then a second finger: nothing drawn.
    page.click(".s-tool[data-tool=line]")
    (x1, y1), (x2, y2) = s_at(page, 3, 10), s_at(page, 12, 10)
    cdp.send("Input.dispatchTouchEvent", {"type": "touchStart", "touchPoints": [pt(x1, y1, 1)]})
    cdp.send("Input.dispatchTouchEvent", {"type": "touchMove", "touchPoints": [pt(x2, y2, 1)]})
    page.wait_for_timeout(50)
    seen = len(s_on(page))
    cdp.send("Input.dispatchTouchEvent", {"type": "touchStart", "touchPoints": [pt(x2, y2, 1), pt(x2 + 40, y2, 2)]})
    cdp.send("Input.dispatchTouchEvent", {"type": "touchEnd", "touchPoints": []})
    page.wait_for_timeout(100)
    check(seen == 10 and not s_on(page) and s_steps(page) == "0/0", f"/studio/ (phone): a line seen as it is dragged ({seen}), gone when a second finger lands")
    # Pinch out: zoom in about the fingers; pinch in: back to fit.
    z0 = page.inner_text("#s-z")
    cx, cy = 195, s_at(page, 8, 16)[1]
    cdp.send("Input.dispatchTouchEvent", {"type": "touchStart", "touchPoints": [pt(cx - 30, cy, 1)]})
    cdp.send("Input.dispatchTouchEvent", {"type": "touchStart", "touchPoints": [pt(cx - 30, cy, 1), pt(cx + 30, cy, 2)]})
    for d in range(30, 130, 10):
        cdp.send("Input.dispatchTouchEvent", {"type": "touchMove", "touchPoints": [pt(cx - d, cy, 1), pt(cx + d, cy, 2)]})
    cdp.send("Input.dispatchTouchEvent", {"type": "touchEnd", "touchPoints": []})
    z1 = page.inner_text("#s-z")
    cdp.send("Input.dispatchTouchEvent", {"type": "touchStart", "touchPoints": [pt(cx - 150, cy, 1)]})
    cdp.send("Input.dispatchTouchEvent", {"type": "touchStart", "touchPoints": [pt(cx - 150, cy, 1), pt(cx + 150, cy, 2)]})
    for d in range(150, 5, -10):
        cdp.send("Input.dispatchTouchEvent", {"type": "touchMove", "touchPoints": [pt(cx - d, cy, 1), pt(cx + d, cy, 2)]})
    cdp.send("Input.dispatchTouchEvent", {"type": "touchEnd", "touchPoints": []})
    z2 = page.inner_text("#s-z")
    wide = page.evaluate("document.documentElement.scrollWidth - innerWidth")
    check(z0 == "8×" and z1 not in ("8×", "FIT") and int(z1.rstrip("×")) > 8 and z2 == "FIT" and not s_on(page) and wide <= 0,
          f"/studio/ (phone): two fingers pinch the zoom, out and in ({z0} -> {z1} -> {z2}), drawing nothing, no sideways scroll")
    check(not page.evaluate("window.__csp || []"), f"/studio/ (phone): no CSP violation {page.evaluate('window.__csp || []')[:2]}")
    page.context.close()


# --- the Studio, S2 branch 1 (studio-s2-doc): the model, the split, the review items ---
# What a first visit to /studio/ fetches, and nothing else: the seven
# first-paint files, the site's stylesheet, fonts and wordmark, the starter pack.
S_FIRST = {"/studio/", "/studio/studio.css", "/studio/studio.js", "/studio/doc.js", "/studio/store.js", "/studio/palettes.js",
           "/studio/tools.js", "/studio/starter.dmdpack", "/site.css", "/fonts/fraunces.woff2", "/fonts/silkscreen.woff2", "/art/wordmark.png"}


def s_paths(page: Page) -> set[str]:
    return {urllib.parse.urlsplit(u).path for u in page.asked}


def studio_fetches(browser, base: str, site: Path, errors: list[str]) -> None:
    """A first visit fetches first paint and nothing else; each lazy module only at its trigger."""
    page = s_page(browser, base, site, errors)
    wait_js(page, "document.getElementById('studio').dataset.starter !== undefined", timeout=20000)
    page.wait_for_timeout(300)
    got = s_paths(page)
    check(got == S_FIRST, f"/studio/ fetches: a first visit asks for first paint and nothing else (extra {sorted(got - S_FIRST)}, missing {sorted(S_FIRST - got)})")
    page.mouse.click(*s_at(page, 4, 4))
    s_kept(page)
    page.click("#s-add")
    wait_js(page, "!document.getElementById('s-picker').hidden && document.getElementById('s-picker').firstChild !== null")
    now = s_paths(page)
    check("/studio/colour.js" not in got and "/studio/colour.js" in now and "/studio/out.js" not in now
          and not any(p.startswith("/demo/") for p in now),
          "/studio/ fetches: colour.js only when New colour… opens; out.js and the GIF writer not before a download")
    with page.expect_download(timeout=60000):
        page.click("#s-gif")
    after = s_paths(page)
    check("/studio/out.js" in after and any("pkg-studio" in p for p in after) and not any("pkg-art" in p for p in after),
          "/studio/ fetches: out.js and pkg-studio at the first download")
    page.context.close()


# A piece of three shown layers and a hidden fourth over four frames, colours
# k1..k4; the Python twin of what the page must show.
S_LAYERED = """async () => { const m = await import('/studio/store.js'); const s = await m.LocalStore.open();
    // Cels as stored deflate blocks (zlib, level 0), not as this browser would
    // pack them: one packed again shows as a different string, the same hash.
    const stored = (a) => { const z = new Uint8Array(2 + 5 + a.length + 4); let s1 = 1, s2 = 0;
      z.set([0x78, 0x01, 0x01, a.length & 255, a.length >> 8, ~a.length & 255, (~a.length >> 8) & 255]); z.set(a, 7);
      for (const x of a) { s1 = (s1 + x) % 65521; s2 = (s2 + s1) % 65521; }
      z.set([s2 >> 8, s2 & 255, s1 >> 8, s1 & 255], 7 + a.length);
      let t = ''; for (const x of z) t += String.fromCharCode(x); return btoa(t); };
    const cel = async (f) => { const a = new Uint8Array(4096); for (let i = 0; i < 4096; i++) a[i] = f(i % 128, (i / 128) | 0); return stored(a); };
    const rocket = await Promise.all([0, 1, 2, 3].map((n) => cel((x, y) => (x >= 10 * n && x < 10 * n + 10 && y < 10 ? 2 : 0))));
    const body = { w: 128, h: 32, palette: 'pl_00000000000070008000000000000001',
      colours: [{ k: 1, hex: '#102030' }, { k: 2, hex: '#405060' }, { k: 3, hex: '#708090' }, { k: 4, hex: '#a0b0c0' }],
      frames: ['f0000001', 'f0000002', 'f0000003', 'f0000004'].map((key) => ({ key, ms: 150 })),
      layers: [
        { key: 'a0000001', name: 'Sky', moves: false, cels: [await cel((x) => (x < 64 ? 1 : 0))] },
        { key: 'a0000002', name: 'Rocket', moves: true, cels: rocket },
        { key: 'a0000003', name: 'Ground', moves: false, cels: [await cel((x, y) => (y === 31 && x < 100 ? 3 : 0))] },
        { key: 'a0000004', name: 'Ghost', moves: false, hide: true, cels: [await cel((x) => (x === 127 || x === 5 ? 4 : 0))] }] };
    const h = await s.put(m.record('piece', 'Layered', body));
    s.db.close();
    localStorage.setItem('odmd.studio.last', h.id);
    return h; }"""


def py_layered(f: int) -> list[tuple]:
    c = {0: (0, 0, 0), 1: (0x10, 0x20, 0x30), 2: (0x40, 0x50, 0x60), 3: (0x70, 0x80, 0x90)}
    out = []
    for y in range(32):
        for x in range(128):
            k = 1 if x < 64 else 0
            if 10 * f <= x < 10 * f + 10 and y < 10:
                k = 2
            if y == 31 and x < 100:
                k = 3
            out.append(c[k])
    return out


def studio_layers(browser, base: str, site: Path, errors: list[str]) -> None:
    """A many-layer, many-frame piece put straight into the store: shown as
    composited, saved back with the same hash, downloaded frame by frame
    with its hidden layer left out; an S1 journal dropped at open."""
    page = s_page(browser, base, site, errors)
    h = page.evaluate(S_LAYERED)
    page.reload()
    page.wait_for_selector(S_READY, timeout=30000)
    shown = s_pixels(page)
    check(s_data(page).get("id") == h["id"] and shown == py_layered(0),
          f"/studio/ layers: three layers and a hidden fourth open as their composite on frame 1 ({sum(a != b for a, b in zip(shown, py_layered(0)))} pixels off)")
    before = page.evaluate(S_STORE, ["get", None])["r"]
    page.click('#s-set .s-sw[data-hex="#f7a531"]')
    page.mouse.click(*s_at(page, 90, 20))
    s_kept(page)
    drawn = page.evaluate(S_STORE, ["get", None])["r"]
    page.keyboard.press("Control+z")
    s_kept(page)
    back = page.evaluate(S_STORE, ["get", None])["r"]
    # Ground (layers[2], the top shown layer) was drawn on and packed again; the others are as stored.
    same = [back["body"]["layers"][n]["cels"] == before["body"]["layers"][n]["cels"] for n in (0, 1, 3)]
    check(drawn["hash"] != h["hash"] and back["hash"] == h["hash"] == s_hash(back["body"]) and all(same)
          and back["body"]["layers"][3].get("hide") is True and "hide" not in back["body"]["layers"][0],
          f"/studio/ layers: drawn on and undone, it saves back with the same hash; the cels not drawn on are written as they were, not packed again {same}")
    with page.expect_download(timeout=60000) as dl:
        page.click("#s-gif")
    im = Image.open(io.BytesIO(Path(dl.value.path()).read_bytes()))
    frames, delays = [], []
    try:
        while True:
            frames.append(list(im.convert("RGB").getdata()))
            delays.append(im.info.get("duration"))
            im.seek(im.tell() + 1)
    except EOFError:
        pass
    check(len(frames) == 4 and all(frames[f] == py_layered(f) for f in range(4)) and delays == [150] * 4,
          f"/studio/ layers: its GIF has every frame composited, the hidden layer left out ({len(frames)} frames, {delays})")
    # A whole piece handed in (studio.newPiece, piece()): its layers, frames, tags and from as given; a bad one refused.
    whole = page.evaluate("""async () => { const m = await import('/studio/studio.js');
        const cel = (k) => { const a = new Uint8Array(4096); a.fill(k, 0, 128); return a; };
        await m.piece({ name: 'Whole', tags: ['imported'], plays: 2, colours: [{ k: 1, hex: '#102030' }, { k: 2, hex: '#405060' }],
          from: [{ external: true, note: 'Game art: Example', title: 'Example', platform: 'SNES' }], frames: [{ ms: 100 }, { ms: 200 }],
          layers: [{ name: 'Back', moves: true, cels: [cel(1), cel(2)] }, { name: 'Hid', moves: false, hide: true, cels: [cel(2)] }] });
        let bad; try { await m.piece({ frames: [{ ms: 0 }] }); bad = 'taken'; } catch (e) { bad = e.message; }
        return bad; }""")
    page.mouse.click(*s_at(page, 64, 20))
    s_kept(page)
    w = page.evaluate(S_STORE, ["get", None])["r"]
    wb = w["body"]
    check(w["name"] == "Whole" and w["tags"] == ["imported"] and w["from"][0].get("platform") == "SNES" and wb["plays"] == 2
          and [f["ms"] for f in wb["frames"]] == [100, 200] and [(l["name"], l["moves"], l.get("hide"), len(l["cels"])) for l in wb["layers"]] == [("Back", True, None, 2), ("Hid", False, True, 1)]
          and all(re.fullmatch(r"[0-9a-f]{8}", x["key"]) for x in wb["frames"] + wb["layers"]) and "not a record" in whole,
          f"/studio/ layers: a whole piece handed in keeps its layers, frames, tags and from; one that lies is refused ({whole})")
    page.context.close()


def studio_old_journal(browser, base: str, site: Path, errors: list[str]) -> None:
    """An S1 draft with an S1 journal (whole colour lists, no cel named): its
    pixels as stored, its undo history dropped once, the record untouched."""
    page = s_page(browser, base, site, errors)
    old = page.evaluate("""async () => { const m = await import('/studio/store.js'); const s = await m.LocalStore.open();
        const px = new Uint8Array(4096); px[3] = 1;
        const body = { w: 128, h: 32, plays: 0, palette: 'pl_00000000000070008000000000000001', colours: [{ k: 1, hex: '#f7a531' }],
          frames: [{ key: '0000000a', ms: 5000 }], layers: [{ key: '0000000b', name: 'Drawing', moves: false, cels: [await m.packPx(px)] }] };
        const h = await s.put(m.record('piece', 'S1 draft', body), { at: 0 });
        await s.jot(h.id, { step: { n: 1, i: Int32Array.of(9), b: Uint16Array.of(0), a: Uint16Array.of(2),
          cb: [{ k: 1, hex: '#f7a531' }], ca: [{ k: 1, hex: '#f7a531' }, { k: 2, hex: '#33d1ff' }] }, cur: 1 });
        s.db.close(); localStorage.setItem('odmd.studio.last', h.id); return h; }""")
    page.reload()
    page.wait_for_selector(S_READY, timeout=30000)
    page.wait_for_timeout(300)
    left = page.evaluate("""async (id) => { const m = await import('/studio/store.js'); const s = await m.LocalStore.open();
        const j = await s.steps(id), r = await s.get(id); s.db.close(); return [j.list.length, r.hash]; }""", old["id"])
    check(s_data(page)["id"] == old["id"] and s_on(page) == {(3, 0)} and s_steps(page) == "0/0" and left == [0, old["hash"]],
          f"/studio/ layers: an S1 draft with an S1 journal opens with its pixels as stored and no undo history ({sorted(s_on(page))}, steps {s_steps(page)}, {left[0]} steps left)")
    # Drawn on, it is written in the form the rule asks: its plays 0 (forever) as no plays.
    page.mouse.click(*s_at(page, 30, 30))
    s_kept(page)
    b = page.evaluate(S_STORE, ["get", None])["r"]["body"]
    check("plays" not in b and "keys" not in b, f"/studio/ layers: an S1 draft's plays 0 is written as no plays at its next save ({sorted(b)})")
    page.context.close()


def studio_shade(browser, base: str, site: Path, errors: list[str]) -> None:
    """Shade: Solid, Light, Half, Heavy, for Fill and the filled shapes, anchored to the panel."""
    page = s_page(browser, base, site, errors)
    s_blur(page)
    want = {"light": lambda x, y: x % 2 == 0 and y % 2 == 0, "half": lambda x, y: (x + y) % 2 == 0,
            "heavy": lambda x, y: not (x % 2 and y % 2), "solid": lambda x, y: True}
    page.keyboard.press("g")
    counts = {}
    for name in ("light", "half", "heavy", "solid"):
        page.click(f"#s-shade [data-shade={name}]")
        page.mouse.click(*s_at(page, 64, 16))
        got = s_on(page)
        counts[name] = (len(got), got == {(x, y) for x in range(128) for y in range(32) if want[name](x, y)})
        page.keyboard.press("Control+z")
    check(counts == {"light": (1024, True), "half": (2048, True), "heavy": (3072, True), "solid": (4096, True)},
          f"/studio/ shade: Light, Half, Heavy and Solid fill 1,024, 2,048, 3,072 and 4,096 of 4,096, as patterns of the panel {counts}")
    # Two filled rectangles side by side continue one pattern (anchored at the panel's 0, 0, not each shape's).
    page.click("#s-shade [data-shade=half]")
    page.keyboard.press("Shift+R")
    s_drag(page, (11, 5), (19, 14))
    s_drag(page, (20, 5), (30, 14))
    got = s_on(page)
    check(got == {(x, y) for x in range(11, 31) for y in range(5, 15) if (x + y) % 2 == 0},
          f"/studio/ shade: two filled rectangles at Half, side by side, continue one pattern ({len(got)} lit)")
    s_new(page)
    page.click("#s-shade [data-shade=light]")
    page.keyboard.press("Shift+O")
    s_drag(page, (3, 3), (21, 17))
    lit = s_on(page)
    page.keyboard.press("o")  # the edge: every pixel, whatever the shade
    s_drag(page, (40, 3), (58, 17))
    edge = s_on(page) - lit
    check(lit and all(x % 2 == 0 and y % 2 == 0 for x, y in lit) and lit <= py_ellipse(3, 3, 21, 17, True) and edge == py_ellipse(40, 3, 58, 17),
          f"/studio/ shade: a filled ellipse takes the shade, an edge does not ({len(lit)}, {len(edge)})")
    seen = []
    page.keyboard.press("g")
    page.click("#s-shade [data-shade=solid]")
    for _ in range(4):
        page.keyboard.press("d")
        seen.append(s_data(page).get("shade"))
    vis = {}
    for k in ("b", "g", "r", "Shift+R", "o", "Shift+O", "m"):
        page.keyboard.press(k)
        vis[k] = page.is_visible("#s-shade")
    check(seen == ["light", "half", "heavy", "solid"] and vis == {"b": False, "g": True, "r": False, "Shift+R": True, "o": False, "Shift+O": True, "m": False},
          f"/studio/ shade: D cycles the four ({seen}); shown only for Fill and the filled shapes {vis}")
    page.context.close()


def s_colours_ok(page: Page) -> tuple[bool, list]:
    cs = page.evaluate(S_STORE, ["get", None])["r"]["body"]["colours"]
    return len({c["hex"] for c in cs}) == len(cs) and len({c["k"] for c in cs}) == len(cs), cs


def studio_two_keys(browser, base: str, site: Path, errors: list[str]) -> None:
    """Draw, undo, re-pick, redo, draw (and the same with a paste floating):
    one colour, one key; the draft goes on saving."""
    page = s_page(browser, base, site, errors)
    s_blur(page)
    cyan = '#s-set .s-sw[data-hex="#33d1ff"]'
    page.click(cyan)
    page.mouse.click(*s_at(page, 1, 1))
    s_kept(page)
    page.keyboard.press("Control+z")
    s_kept(page)
    page.click(cyan)
    page.keyboard.press("Control+Shift+z")
    page.mouse.click(*s_at(page, 2, 2))
    s_kept(page)
    ok, cs = s_colours_ok(page)
    check(ok and s_data(page)["saved"] == "kept" and [c["hex"] for c in cs].count("#33d1ff") == 1 and s_px(page, 1, 1) == s_px(page, 2, 2) == (0x33, 0xd1, 0xff),
          f"/studio/ two keys: draw, undo, re-pick, redo, draw: one colour under one key, still saving ({cs}, {s_data(page)['saved']})")
    # The same with a paste floating over the redo.
    s_new(page)
    page.click(cyan)
    page.mouse.click(*s_at(page, 1, 1))
    page.keyboard.press("m")
    s_drag(page, (0, 0), (2, 2))
    page.keyboard.press("Control+c")
    page.keyboard.press("Enter")
    page.keyboard.press("b")
    s_kept(page)
    page.keyboard.press("Control+z")
    page.click('#s-set .s-sw[data-hex="#ff4f8b"]')
    page.click(cyan)
    page.keyboard.press("Control+v")
    page.keyboard.press("Control+Shift+z")
    page.keyboard.press("Enter")
    page.keyboard.press("b")
    page.mouse.click(*s_at(page, 30, 20))
    s_kept(page)
    ok, cs = s_colours_ok(page)
    check(ok and s_data(page)["saved"] == "kept" and [c["hex"] for c in cs] == ["#33d1ff"] and len(s_on(page)) == 3,
          f"/studio/ two keys: the same with a paste floating over the redo: one colour, one key, still saving ({cs}, {len(s_on(page))} lit)")
    page.context.close()


# doc.js driven with no page controls: random draws, undos, redos, picks and
# pastes; after each, no colour twice and no key twice; now and then the
# journal replayed from the last record saved gives the same pixels and colours.
S_MODEL = """async ([seed, ops]) => { const { Doc, N } = await import('/studio/doc.js'); const m = await import('/studio/store.js');
    let r = seed >>> 0;
    const rnd = (n) => { r = (r + 0x6d2b79f5) >>> 0; let t = Math.imul(r ^ (r >>> 15), 1 | r); t ^= t + Math.imul(t ^ (t >>> 7), 61 | t); return ((t ^ (t >>> 14)) >>> 0) % n; };
    const body = { w: 128, h: 32, plays: 0, colours: [], frames: [{ key: '00000001', ms: 100 }],
      layers: [{ key: '00000002', name: 'a', moves: false, cels: [await m.packPx(new Uint8Array(N))] }] };
    const d = new Doc(body); await d.load();
    let float = null; d.held = () => float;
    const pal = [0x112233, 0x445566, 0x778899, 0xaabbcc, 0xddeeff, 0x010203, 0xf7a531];
    let saved = { body, at: 0 }, replays = 0;
    const bad = [];
    const same = (a, b) => a.length === b.length && a.every((x, i) => x === b[i]);
    const save = async () => { saved = { body: await d.packed(d.snap()), at: d.low + d.cur - 1 }; };
    for (let op = 0; op < ops; op++) {
      const t = rnd(7);
      try {
        if (t === 0 || t === 1) {
          if (float && rnd(2)) { d.begin(); const at = rnd(N - 8); float.forEach((k, j) => { if (k) d.set(at + j, k); }); float = null; }
          else { if (!d.key) d.pickColour(pal[rnd(pal.length)]); d.begin(); for (let n = 0, c = 1 + rnd(12); n < c; n++) d.set(rnd(N), rnd(6) ? d.key : 0); }
          const w = d.end();
          if (w && (rnd(3) || saved.at >= w.step.n || saved.at < d.low - 1)) await save();
        } else if (t === 2) { if (float) { float = null; d.letGo(d.key); } else await d.step(false); }
        else if (t === 3) await d.step(true);
        else if (t === 4) d.pickColour(pal[rnd(pal.length)]);
        else if (t === 5) float = Uint16Array.from(d.keysFor(Array.from({ length: 1 + rnd(5) }, () => (rnd(4) ? pal[rnd(pal.length)] : 0x100000 + rnd(9)))));
        else if (t === 6) { const w = await d.unused(); if (w) await save(); }
      } catch (e) { bad.push(`${op}: ${e.message}`); }
      const l = d.colourList(), vals = new Set(d.cols.values());
      if (new Set(l.map((c) => c.hex)).size !== l.length || [...d.pending].some(([k, c]) => d.cols.has(k) || vals.has(c))) bad.push(`${op}: colours`);
      if (op % 10 === 9 && !d.stroke) {
        const e = await Doc.open({ hash: 'h', body: saved.body }, undefined, { list: d.steps, cur: d.low + d.cur - 1, at: saved.at, hash: 'h' });
        if (e.said === 'dropped' || !same(e.px, d.px) || JSON.stringify(e.colourList()) !== JSON.stringify(d.colourList())) bad.push(`${op}: replay ${e.said}`);
        replays++;
      }
    }
    return { bad: bad.slice(0, 5), nbad: bad.length, replays, steps: d.steps.length, colours: d.cols.size };
}"""

S_PASTE4000 = """async () => { const { Doc, N } = await import('/studio/doc.js'); const m = await import('/studio/store.js');
    const d = new Doc({ w: 128, h: 32, plays: 0, colours: [], frames: [{ key: '00000001', ms: 100 }],
      layers: [{ key: '00000002', name: 'a', moves: false, cels: [await m.packPx(new Uint8Array(N))] }] });
    await d.load();
    for (let n = 0; n < 520; n++) { d.pickColour(0x200000 + (n % 90)); d.begin(); d.set(n, d.key); d.end(); }
    const list = Array.from({ length: 4000 }, (_, n) => 0x400000 + n);
    d.stats.usedBuilds = 0;
    const t = performance.now(), ks = d.keysFor(list), ms = performance.now() - t;
    return { steps: d.steps.length, builds: d.stats.usedBuilds, ms, distinct: new Set(ks).size, zero: ks.filter((k) => !k).length, pending: d.pending.size, cols: d.cols.size };
}"""


def studio_model(browser, base: str, site: Path, errors: list[str]) -> None:
    """doc.js alone: 2,000 random operations keep the colours unique and the
    journal replayable; a paste of 4,000 new colours over a full journal
    gives out keys from one allocator."""
    page = s_page(browser, base, site, errors)
    res = [page.evaluate(S_MODEL, [seed, 2000 if seed == 1 else 700]) for seed in (1, 2, 3)]
    check(all(x["nbad"] == 0 and x["replays"] >= 70 for x in res),
          f"/studio/ model: 2,000 random draws, undos, redos, picks, pastes and removals (and two more runs of 700): colours unique after each, the journal replayed to the same pixels {res}")
    p = page.evaluate(S_PASTE4000)
    check(p["steps"] == 500 and p["builds"] == 1 and p["ms"] < 100 and p["distinct"] == 4000 and p["zero"] == 0 and p["pending"] == 4000,
          f"/studio/ model: 4,000 new colours pasted over a 500-step journal: keys from one build of the used set, in {p['ms']:.1f} ms {p}")
    page.context.close()


# Hostile input for the bounded inflate: every DecompressionStream the page
# makes counted by what it gives (whoever reads it), then cels that lie.
S_BOMBS = """async () => { const m = await import('/studio/store.js');
    const Real = DecompressionStream; let given = 0;
    window.DecompressionStream = class { constructor(f) { const d = new Real(f); this.writable = d.writable;
      this.readable = d.readable.pipeThrough(new TransformStream({ transform(c, k) { given += c.length; k.enqueue(c); } })); } };
    const zeros = (n) => m.packPx(new Uint8Array(n));
    const b64 = (u) => { let s = ''; for (const x of u) s += String.fromCharCode(x); return btoa(s); };
    const raw = (s) => Uint8Array.from(atob(s), (c) => c.charCodeAt(0));
    const out = {};
    const run = async (name, cel, n, wide) => {
      given = 0; const t = performance.now(); let said;
      try { const px = await m.unpackPx(cel, n, wide); said = `taken ${px.length}`; } catch (e) { said = e instanceof Error && !(e instanceof TypeError) ? e.message : `threw ${e}`; }
      await new Promise((ok) => setTimeout(ok, 50));
      out[name] = { said, given, kept: m.inflated?.kept ?? -1, ms: Math.round(performance.now() - t) };
    };
    const good = await zeros(4096), z = raw(good);
    await run('ok', good, 4096);
    await run('bomb 100 MB', await zeros(100 << 20), 4096);
    await run('bomb 4 MiB, under the text cap', await zeros(4 << 20), 4096);
    await run('bomb 8 MiB two-byte', await zeros(8 << 20), 4096, true);
    await run('one byte short', await zeros(4095), 4096);
    await run('one byte long', await zeros(4097), 4096);
    await run('one-byte cel read as two', good, 4096, true);
    await run('truncated', b64(z.subarray(0, z.length - 3)), 4096);
    const f = z.slice(); f[2] ^= 0xff; await run('bit-flipped', b64(f), 4096);
    await run('not base64', '!!not base64!!', 4096);
    await run('empty', '', 4096);
    await run('not a string', 12345, 4096);
    await run('raw deflate, no zlib header', b64(z.subarray(2)), 4096);
    // The hash's pixel form is bounded too: a put of a piece whose cel is a bomb.
    given = 0;
    const s = await m.LocalStore.open();
    const body = { w: 128, h: 32, plays: 0, frames: [{ ms: 100 }], layers: [{ name: 'a', moves: false, cels: [await zeros(4 << 20)] }] };
    try { await s.put(m.record('piece', 'bomb', body)); out.put = { said: 'taken', given }; } catch (e) { out.put = { said: e.message, given }; }
    s.db.close();
    window.DecompressionStream = Real;
    return out;
}"""

S_OPEN_BOMB = """async () => { const m = await import('/studio/store.js'); const s = await m.LocalStore.open();
    const bomb = await m.packPx(new Uint8Array(4 << 20)), id = m.newId('piece');
    const rec = { ...m.record('piece', 'Bomb', { w: 128, h: 32, plays: 0, frames: [{ ms: 100 }], layers: [{ name: 'a', moves: false, cels: [bomb] }] }), id, hash: 'sha256-' + '0'.repeat(64), seq: 999999 };
    const tx = s.db.transaction('records', 'readwrite'); tx.objectStore('records').put(rec);
    await new Promise((ok) => { tx.oncomplete = ok; });
    s.db.close(); localStorage.setItem('odmd.studio.last', id); return id; }"""


def studio_bombs(browser, base: str, site: Path, errors: list[str]) -> None:
    """The hostile-input list, part 1: deflate bombs and broken cels."""
    page = s_page(browser, base, site, errors)
    b = page.evaluate(S_BOMBS)
    wrong = lambda x: x["said"] in ("a picture of the wrong size", "a picture that does not read")
    check(b["ok"]["said"] == "taken 4096", f"/studio/ hostile: a good cel reads {b['ok']}")
    check(wrong(b["bomb 100 MB"]) and b["bomb 100 MB"]["given"] == 0,
          f"/studio/ hostile: a cel that inflates to 100 MB is refused before a byte is inflated (its text is longer than a cel's can be) {b['bomb 100 MB']}")
    bb = b["bomb 4 MiB, under the text cap"]
    check(wrong(bb) and bb["kept"] <= 4097 and bb["given"] <= 256 * 1024,
          f"/studio/ hostile: a cel that inflates to 4 MiB from text short enough to pass is refused at 4,097 bytes kept, the stream dropped after {bb['given']:,} B (one browser chunk) {bb}")
    bw = b["bomb 8 MiB two-byte"]
    check(wrong(bw) and bw["kept"] <= 8193 and bw["given"] <= 256 * 1024, f"/studio/ hostile: the same for a two-byte cel {bw}")
    rest = {k: v["said"] for k, v in b.items() if k not in ("ok", "bomb 100 MB", "bomb 4 MiB, under the text cap", "bomb 8 MiB two-byte", "put")}
    check(all(wrong(b[k]) for k in rest), f"/studio/ hostile: broken cels refused with a sentence, never a TypeError {rest}")
    check(b["put"]["said"] != "taken" and b["put"]["given"] <= 256 * 1024, f"/studio/ hostile: a put whose cel is a bomb is refused, its hash bounded too {b['put']}")
    # A stored draft whose cel is a bomb (written past check()): said, and a new piece opens.
    page.evaluate(S_OPEN_BOMB)
    page.reload()
    page.wait_for_selector(S_READY, timeout=30000)
    check(page.is_visible("#s-err") and "damaged" in page.inner_text("#s-err") and s_data(page)["saved"] == "new",
          f"/studio/ hostile: a stored draft whose cel is a bomb is said to be damaged, and a new piece opens ('{page.inner_text('#s-err')[:50]}')")
    page.context.close()


def studio_records(browser, base: str, site: Path, errors: list[str]) -> None:
    """Records that lie: each refused by check() with a sentence; the good ones taken."""
    page = s_page(browser, base, site, errors)
    r = page.evaluate("""async () => { const m = await import('/studio/store.js');
        const cel = await m.packPx(new Uint8Array(4096));
        const base = () => ({ ...m.record('piece', 'x', { w: 128, h: 32, plays: 0, palette: 'pl_x', colours: [{ k: 1, hex: '#123456' }],
          frames: [{ key: '0000000f', ms: 100 }], layers: [{ key: '000000aa', name: 'a', moves: false, cels: [cel] }] }) });
        const L = (o) => (r) => { Object.assign(r.body.layers[0], o); };
        const F = (o) => (r) => { Object.assign(r.body.frames[0], o); };
        const cases = {
          'hide false': L({ hide: false }), 'hide "yes"': L({ hide: 'yes' }), 'moves "no"': L({ moves: 'no' }),
          'a layer key twice': (r) => { r.body.layers.push({ ...r.body.layers[0] }); },
          'a layer key the same as a frame key': L({ key: '0000000f' }),
          'a key in capitals': L({ key: '000000AA' }), 'a key of seven digits': F({ key: '0000000' }), 'a key that is a number': F({ key: 15 }),
          'ms 0': F({ ms: 0 }), 'ms 1.5': F({ ms: 1.5 }), 'ms 19': F({ ms: 19 }), 'ms 60001': F({ ms: 60001 }), 'ms "100"': F({ ms: '100' }), 'no ms': (r) => { delete r.body.frames[0].ms; },
          'a layer name of 61': L({ name: 'n'.repeat(61) }), 'a layer name that is not a string': L({ name: 5 }),
          'a cel that is a number': L({ cels: [5] }), 'two cels on a held layer': L({ cels: [cel, cel] }),
          'a layer that is null': (r) => { r.body.layers = [null]; }, 'a frame that is null': (r) => { r.body.frames = [null]; },
          'no frames': (r) => { r.body.frames = []; }, '257 frames': (r) => { r.body.frames = Array.from({ length: 257 }, () => ({ ms: 100 })); },
          '17 layers': (r) => { r.body.layers = Array.from({ length: 17 }, () => ({ name: 'a', moves: false, cels: [cel] })); },
          'plays -1': (r) => { r.body.plays = -1; }, 'plays 1.5': (r) => { r.body.plays = 1.5; }, 'keys 3': (r) => { r.body.keys = 3; },
          'a 5 MiB piece': L({ cels: ['A'.repeat(5 << 20)] }), 'body null': (r) => { r.body = null; }, 'body an array': (r) => { r.body = []; },
          'a colour twice': (r) => { r.body.colours.push({ k: 2, hex: '#123456' }); }, 'colour key 0': (r) => { r.body.colours[0].k = 0; },
          'colour key 256 in a one-byte piece': (r) => { r.body.colours[0].k = 256; }, 'a colour in capitals': (r) => { r.body.colours[0].hex = '#ABCDEF'; },
          'a colour that is null': (r) => { r.body.colours = [null]; },
          'type constructor': (r) => { r.type = 'constructor'; }, 'type __proto__': (r) => { r.type = '__proto__'; }, 'another type\\'s id': (r) => { r.id = 'pl' + r.id.slice(2); },
          'from naming a record without its hash': (r) => { r.from = [{ id: 'pc_' + '1'.repeat(32) }]; }, 'from of 65': (r) => { r.from = Array.from({ length: 65 }, () => ({ external: true, note: '' })); },
          'a licence outside the list': (r) => { r.licence = 'MIT'; }, 'author null': (r) => { r.author = null; }, 'tags a string': (r) => { r.tags = 'x'; },
          'a note of 201': (r) => { r.note = 'n'.repeat(201); }, 'rev 0': (r) => { r.rev = 0; }, 'the record null': () => null, 'the record a string': () => 'piece',
          // review round (Fable): refs of the wrong shape, palettes with no colours, holes, the written form
          'layer stamps 5': L({ stamps: 5 }), 'layer stamps [null]': L({ stamps: [null] }), 'layer stamps {}': L({ stamps: {} }), 'a stamp with no ref': L({ stamps: [{}] }),
          'members 5': (r) => { r.body.members = 5; }, 'members [null]': (r) => { r.body.members = [null]; },
          'a palette with no colours': () => ({ ...m.record('palette', 'p', {}) }), 'a palette whose colours are a string': () => ({ ...m.record('palette', 'p', { colours: 'red' }) }),
          'keys 1 (written as no keys)': (r) => { r.body.keys = 1; }, 'colours out of key order': (r) => { r.body.colours = [{ k: 9, hex: '#000009' }, { k: 2, hex: '#000002' }]; },
          'an empty note': (r) => { r.note = ''; }, 'frames with a hole': (r) => { r.body.frames = new Array(1); }, 'cels with a hole': L({ cels: new Array(1) }),
          'layers with a hole': (r) => { r.body.layers = new Array(1); }, 'colours with a hole': (r) => { r.body.colours = new Array(2); }, 'tags with a hole': (r) => { r.tags = new Array(2); },
          'from with a hole': (r) => { r.from = new Array(2); }, 'rev 1e308': (r) => { r.rev = 1e308; }, 'created an object': (r) => { r.created = {}; }, 'palette a number': (r) => { r.body.palette = 5; },
        };
        const out = {};
        for (const [name, f] of Object.entries(cases)) {
          const r = base(), r2 = f(r);
          try { m.check(r2 === undefined ? r : r2); out[name] = 'taken'; } catch (e) { out[name] = e instanceof TypeError ? 'TypeError ' + e.message : e.message.startsWith('not a record:') ? 'refused' : e.message; }
        }
        const good = {};
        const ok = (name, f) => { const r = base(); f(r); try { m.check(r); good[name] = 'taken'; } catch (e) { good[name] = e.message; } };
        ok('as made', () => {});
        ok('hide true', L({ hide: true }));
        ok('a 2 MiB piece (the cap is 4 MiB)', L({ cels: ['A'.repeat(2 << 20)] }));
        ok('no keys (pre-S1)', (r) => { delete r.body.frames[0].key; delete r.body.layers[0].key; });
        ok('an external from with the importer\\'s keys', (r) => { r.from = [{ external: true, note: 'Game art: Example', title: 'Example', platform: 'SNES', author: 'someone', logo: true }]; });
        ok('plays 0, as S0 and S1 drafts carry it', (r) => { r.body.plays = 0; });
        ok('stamps as references', L({ stamps: [{ ref: 'st_' + '1'.repeat(32) }] }));
        ok('a from naming a record with its hash and more', (r) => { r.from = [{ id: 'st_' + '1'.repeat(32), rev: 2, hash: 'sha256-' + 'a'.repeat(64), name: 'Star', author: { name: '', account: null }, licence: null }]; });
        const big = { type: 'palette', id: m.newId('palette'), rev: 1, hash: '', name: 'p', tags: [], author: { name: '', account: null }, licence: null, from: [],
          created: '2026-11-11T00:00:00Z', modified: '2026-11-11T00:00:00Z', body: { colours: [], source: 'x'.repeat(2 << 20) } };
        try { m.check(big); out['a 2 MiB palette (other records stay at 1 MiB)'] = 'taken'; } catch (e) { out['a 2 MiB palette (other records stay at 1 MiB)'] = 'refused'; }
        // A record parsed from text with __proto__ and constructor keys pollutes nothing.
        const text = JSON.stringify(base()).replace('{"type"', '{"__proto__":{"polluted":1},"constructor":{"prototype":{"polluted":1}},"type"');
        let parsed; try { m.check(JSON.parse(text)); parsed = 'taken'; } catch (e) { parsed = e.message; }
        // put(): what check() refuses is refused before any picture is inflated, and nothing is stored;
        // a cel of the wrong length is refused (the length is known as it is hashed).
        const s = await m.LocalStore.open('smoke-put-' + Math.random().toString(36).slice(2));
        const Real = DecompressionStream; let streams = 0;
        window.DecompressionStream = function (f) { streams++; return new Real(f); };
        const puts = {};
        const tryPut = async (name, f) => { const r = base(); f(r); streams = 0; let said;
          try { await s.put(r); said = 'taken'; } catch (e) { said = e.message; } puts[name] = { said, streams, stored: !!(await s.get(r.id)) }; };
        await tryPut('stamps 5', L({ stamps: 5 }));
        await tryPut('257 frames of distinct cels', (r) => { r.body.frames = Array.from({ length: 257 }, () => ({ ms: 100 })); r.body.layers[0].moves = true; r.body.layers[0].cels = Array(257).fill(cel); });
        { const r = base(); r.body.layers[0].cels = [await m.packPx(new Uint8Array(4095))]; streams = 0; let said;
          try { await s.put(r); said = 'taken'; } catch (e) { said = e.message; } puts['a cel of 4,095 bytes'] = { said, streams, stored: !!(await s.get(r.id)) }; }
        { const many = []; const a = new Uint8Array(64); for (let i = 0; i < 4400; i++) { a[0] = i & 255; a[1] = i >> 8; many.push({ px: await m.packPx(a) }); }
          const r = base(); r.body.extra = many; streams = 0; const t = performance.now(); let said;
          try { await s.put(r); said = 'taken'; } catch (e) { said = e.message; } puts['4,400 pictures in an unknown field'] = { said, streams, stored: !!(await s.get(r.id)), ms: Math.round(performance.now() - t) }; }
        window.DecompressionStream = Real;
        s.db.close();
        const pb = base().body, pp = JSON.parse(JSON.stringify(pb).replace('{', '{"__proto__":5,'));
        let proto; try { proto = (await m.hashOf(pb)) !== (await m.hashOf(pp)) ? 'two hashes' : 'one hash'; } catch (e) { proto = e.message; }
        return { out, good, parsed, puts, proto, polluted: ({}).polluted !== undefined || Object.prototype.polluted !== undefined };
    }""")
    refused = {k: v for k, v in r["out"].items() if v != "refused"}
    check(not refused, f"/studio/ hostile: {len(r['out'])} records that lie, each refused by check() with a sentence ({refused or 'all refused'})")
    pu = r["puts"]
    check(pu["stamps 5"]["said"].startswith("not a record") and pu["stamps 5"]["streams"] == 0 and not pu["stamps 5"]["stored"]
          and pu["257 frames of distinct cels"]["said"].startswith("not a record") and pu["257 frames of distinct cels"]["streams"] == 0
          and pu["a cel of 4,095 bytes"]["said"] == "a picture of the wrong size" and not pu["a cel of 4,095 bytes"]["stored"]
          and pu["4,400 pictures in an unknown field"]["said"] == "too many pictures" and pu["4,400 pictures in an unknown field"]["streams"] <= 4352
          and not any(v["stored"] for v in pu.values()) and r["proto"] == "two hashes",
          f"/studio/ hostile: put() checks before it hashes (nothing inflated for what check() refuses), refuses a cel of the wrong length and more pictures than a piece holds, stores none of them; an own __proto__ is a key like any {pu} {r['proto']}")
    check(all(v == "taken" for v in r["good"].values()) and not r["polluted"],
          f"/studio/ hostile: the good ones taken (hide true, a 2 MiB piece, no keys, an external from with title, platform, author and logo, a from with its hash); nothing polluted {r['good']} {r['parsed']}")
    page.context.close()


def studio_journals(browser, base: str, site: Path, errors: list[str]) -> None:
    """Journals that lie: each dropped at open, the piece as its record says."""
    page = s_page(browser, base, site, errors)
    j = page.evaluate("""async () => { const { Doc, N } = await import('/studio/doc.js'); const m = await import('/studio/store.js');
        const px = new Uint8Array(N); px[7] = 1;
        const rec = { hash: 'h', body: { w: 128, h: 32, plays: 0, colours: [{ k: 1, hex: '#123456' }], frames: [{ key: '0000000f', ms: 100 }],
          layers: [{ key: '000000aa', name: 'a', moves: false, cels: [await m.packPx(px)] }] } };
        const step = (o = {}) => ({ n: 1, l: '000000aa', f: '0000000f', i: Int32Array.of(9), b: Uint16Array.of(0), a: Uint16Array.of(1), ...o });
        const cases = {
          'an index past the panel': [step({ i: Int32Array.of(4096) })], 'a negative index': [step({ i: Int32Array.of(-1) })],
          'indices as a plain array': [step({ i: [9] })], 'keys as a plain array': [step({ a: [1] })], 'lengths that differ': [step({ b: Uint16Array.of(0, 0) })],
          'a layer it does not have': [step({ l: '000000ab' })], 'a frame it does not have': [step({ f: '0000000e' })],
          'a colour delta with a bad colour': [step({ c: { add: [{ k: 2, hex: 'red' }], del: [] } })], 'a colour delta of key 0': [step({ c: { add: [{ k: 0, hex: '#000000' }], del: [] } })],
          'a colour delta with no del': [step({ c: { add: [] } })], 'S1 colour lists': [{ n: 1, cb: [], ca: [{ k: 2, hex: '#00ff00' }] }],
          'a body step with a short cel list': [{ n: 1, body: { b: rec.body, a: { ...rec.body, layers: [{ name: 'a', moves: true, cels: [] }] } } }],
          'a body step that is null': [{ n: 1, body: null }], 'steps not back to back': [step(), step({ n: 3 })], 'a step that is null': [null],
          'a step n that is a string': [step({ n: '1' })],
          'a colour delta listing a colour twice': [step({ a: Uint16Array.of(2), c: { add: [{ k: 2, hex: '#123456' }], del: [] } })],
          'a pixel of a key with no colour': [step({ a: Uint16Array.of(9999) })],
          'a body step to ms 0': [{ n: 1, body: { b: rec.body, a: { ...rec.body, frames: [{ key: '0000000f', ms: 0 }] } } }],
          'a body step to w 999': [{ n: 1, body: { b: rec.body, a: { ...rec.body, w: 999 } } }],
          'a body step to hide false': [{ n: 1, body: { b: rec.body, a: { ...rec.body, layers: [{ ...rec.body.layers[0], hide: false }] } } }],
          'a body step to a colour twice': [{ n: 1, body: { b: rec.body, a: { ...rec.body, colours: [{ k: 1, hex: '#123456' }, { k: 2, hex: '#123456' }] } } }], 'a step with a cel that does not read': [{ n: 1, body: { b: rec.body, a: { ...rec.body, layers: [{ name: 'a', moves: false, cels: ['!!'] }] } } }],
        };
        const out = {};
        for (const [name, list] of Object.entries(cases)) {
          try { const d = await Doc.open(rec, undefined, { list, cur: list.length, at: 0, hash: 'h' }); out[name] = d.said === 'dropped' && d.px[7] === 1 && d.px[9] === 0 && !d.steps.length ? 'dropped' : `kept (${d.said})`; }
          catch (e) { out[name] = 'threw ' + e.message; }
        }
        for (const [name, j] of Object.entries({ 'at past the steps': { at: 5, cur: 1 }, 'cur past the steps': { at: 0, cur: 5 }, 'another record\\'s hash': { at: 0, cur: 1, hash: 'x' } })) {
          const d = await Doc.open(rec, undefined, { list: [step()], hash: 'h', ...j });
          out[name] = d.said === 'dropped' && !d.steps.length ? 'dropped' : `kept (${d.said})`;
        }
        const d = await Doc.open(rec, undefined, { list: [step()], cur: 1, at: 0, hash: 'h' });
        return { out, good: d.said === 'moved' && d.px[9] === 1 };
    }""")
    kept = {k: v for k, v in j["out"].items() if v != "dropped"}
    check(not kept and j["good"], f"/studio/ hostile: {len(j['out'])} journals that lie, each dropped at open with the piece as stored; a good one replayed ({kept or 'all dropped'})")
    check(not page.evaluate("window.__csp || []"), "/studio/ hostile: no CSP violation")
    page.context.close()


# --- the review round (Fable's red team on studio-s2-doc): each reproduction a check ---
S_REC = """async () => { const m = await import('/studio/store.js'); const s = await m.LocalStore.open(); const d = document.getElementById('studio').dataset;
    const r = await s.get(d.id); const j = await s.steps(d.id); s.db.close(); if (!r) return null;
    const px = await m.unpackPx(r.body.layers[0].cels[0], 4096, r.body.keys === 2), on = {};
    px.forEach((k, i) => { if (k) on[`${i % 128},${(i / 128) | 0}`] = k; });
    return { saved: d.saved, colours: r.body.colours, on, body: r.body, hash: r.hash, j: { n: j.list.length, cur: j.cur, at: j.at, hash: j.hash } }; }"""


def s_canvas(page: Page) -> dict:
    return page.evaluate("""() => { const c = document.getElementById('s-canvas').getContext('2d').getImageData(0, 0, 128, 32).data, on = {};
        for (let i = 0; i < 4096; i++) if (c[i * 4] || c[i * 4 + 1] || c[i * 4 + 2]) on[`${i % 128},${(i / 128) | 0}`] = '#' + [c[i * 4], c[i * 4 + 1], c[i * 4 + 2]].map((x) => x.toString(16).padStart(2, '0')).join(''); return on; }""")


def studio_paste_redo(browser, base: str, site: Path, errors: list[str]) -> None:
    """A redo under a floating paste never takes away a colour the paste holds
    (Remove unused redone; a snapshot restore redone): no pixel without a colour."""
    page = s_page(browser, base, site, errors)
    s_blur(page)
    page.mouse.click(*s_at(page, 10, 10))
    page.click('#s-set .s-sw[data-hex="#33d1ff"]')
    page.mouse.click(*s_at(page, 12, 10))
    page.keyboard.press("m")
    s_drag(page, (12, 10), (13, 11))
    page.keyboard.press("Control+c")
    page.keyboard.press("Delete")
    s_kept(page)
    page.click("#s-unused")
    page.wait_for_timeout(300)
    s_kept(page)
    page.keyboard.press("Control+z")
    page.keyboard.press("Control+v")
    page.keyboard.press("Control+y")
    under = s_canvas(page)
    page.keyboard.press("Enter")
    page.wait_for_timeout(300)
    s_kept(page)
    r = page.evaluate(S_REC)
    listed = {c["k"] for c in r["colours"]}
    orphans = {xy: k for xy, k in r["on"].items() if k not in listed}
    check(not orphans and "#33d1ff" in under.values() and r["saved"] == "kept" and "#33d1ff" in s_canvas(page).values(),
          f"/studio/ review: Remove unused redone under a floating paste keeps the paste's colour; put down, no pixel without a colour ({orphans}, {r['colours']})")
    d = page.evaluate("""async () => { const { Doc, N } = await import('/studio/doc.js'); const m = await import('/studio/store.js');
        const px = new Uint8Array(N); px[0] = 1; px[1] = 2;
        const z = await m.packPx(px), z1 = await m.packPx(Uint8Array.from(px, (k) => (k === 1 ? 1 : 0)));
        const body = (cels, colours) => ({ w: 128, h: 32, colours, frames: [{ key: '0000000a', ms: 100 }], layers: [{ key: '0000000b', name: 'L', moves: false, cels }] });
        const A = { k: 1, hex: '#102030' }, C = { k: 2, hex: '#33d1ff' };
        const d = await Doc.open({ hash: 'h', body: body([z], [A, C]) }, undefined, null);
        let fl = null; d.held = () => fl;
        await d.restore(body([z1], [A]));            // C out of the piece
        await d.step(false);                          // undone: C back
        fl = Uint16Array.from(d.keysFor([0x33d1ff])); // a paste of C, under its key
        const w = d.step(true); if (w) await w;       // the restore redone, the paste floating
        const held = d.rgbOf(fl[0]);
        d.begin(); d.set(100, fl[0]); fl = null; d.end();
        return { held: held.toString(16), cols: d.colourList(), px100: d.px[100] }; }""")
    check(d["held"] == "33d1ff" and {"k": d["px100"], "hex": "#33d1ff"} in d["cols"],
          f"/studio/ review: a snapshot restore redone under a floating paste keeps the paste's colour {d}")
    page.context.close()


def studio_keyless_restore(browser, base: str, site: Path, errors: list[str]) -> None:
    """A snapshot with no layer or frame keys (written before keys) restored,
    drawn on, undone twice, redone twice: the same keys each way."""
    page = s_page(browser, base, site, errors)
    out = page.evaluate("""async () => { const m = await import('/studio/store.js'), { Doc, N } = await import('/studio/doc.js');
        const z = await m.packPx(new Uint8Array(N));
        const keyed = { w: 128, h: 32, colours: [{ k: 1, hex: '#102030' }], frames: [{ key: '0000000a', ms: 100 }], layers: [{ key: '0000000b', name: 'L', moves: false, cels: [z] }] };
        const keyless = { w: 128, h: 32, colours: [{ k: 1, hex: '#102030' }], frames: [{ ms: 100 }], layers: [{ name: 'L', moves: false, cels: [z] }] };
        const d = await Doc.open({ hash: 'h', body: keyed }, (b) => b.colours, null); d.key = 1;
        await d.restore(keyless); d.begin(); d.set(5, 1); d.end();
        const log = [];
        for (const fwd of [false, false, true, true]) { try { const w = d.step(fwd); if (w) await w; log.push('ok'); } catch (e) { log.push(e.message); } }
        return { log, steps: `${d.cur}/${d.steps.length}`, px5: d.px[5] }; }""")
    check(out["log"] == ["ok"] * 4 and out["steps"] == "2/2" and out["px5"] == 1,
          f"/studio/ review: a keyless snapshot restored, drawn on, undone twice and redone twice {out}")
    page.context.close()


def studio_bad_palette(browser, base: str, site: Path, errors: list[str]) -> None:
    """A palette with no colours is refused; one already stored (written past
    check()) is left out, and a piece that names it still opens and saves."""
    page = s_page(browser, base, site, errors)
    said = page.evaluate("""async () => { const m = await import('/studio/store.js'); const s = await m.LocalStore.open();
        const p = m.record('palette', 'Empty', {}); let e; try { await s.put(p); e = 'taken'; } catch (x) { e = x.message; }
        const q = { ...m.record('palette', 'Empty too', {}), hash: 'sha256-' + '0'.repeat(64), seq: 999998 };
        await new Promise((ok) => { const tx = s.db.transaction('records', 'readwrite'); tx.objectStore('records').put(q); tx.oncomplete = ok; });
        const px = new Uint8Array(4096); px[3] = 1;
        const h = await s.put(m.record('piece', 'Names it', { w: 128, h: 32, palette: q.id, colours: [{ k: 1, hex: '#f7a531' }],
          frames: [{ key: '0000000a', ms: 5000 }], layers: [{ key: '0000000b', name: 'D', moves: false, cels: [await m.packPx(px)] }] }));
        s.db.close(); localStorage.setItem('odmd.studio.last', h.id); return [e, h.id]; }""")
    page.reload()
    page.wait_for_selector(S_READY, timeout=30000)
    page.mouse.click(*s_at(page, 20, 20))
    s_kept(page)
    opts = page.eval_on_selector_all("#s-pal option", "els => els.map(e => e.textContent)")
    check(said[0] == "not a record: colours" and s_data(page)["id"] == said[1] and s_on(page) == {(3, 0), (20, 20)}
          and "Empty too" not in opts and page.is_hidden("#s-err"),
          f"/studio/ review: a palette with no colours is refused ({said[0]}); one stored anyway is left out, and the piece naming it opens and saves")
    page.context.close()


def studio_orphan_keys(browser, base: str, site: Path, errors: list[str]) -> None:
    """A two-byte piece of 255 colours or fewer holding a key no colour has
    (300): it opens with that pixel off, saves one byte a pixel, downloads."""
    page = s_page(browser, base, site, errors)
    page.evaluate("""async () => { const m = await import('/studio/store.js'); const s = await m.LocalStore.open();
        const k = new Uint16Array(4096); k[5] = 300; k[6] = 1;
        const h = await s.put(m.record('piece', 'Orphan key', { w: 128, h: 32, keys: 2, colours: [{ k: 1, hex: '#f7a531' }],
          frames: [{ key: '0000000a', ms: 5000 }], layers: [{ key: '0000000b', name: 'D', moves: false, cels: [await m.packPx(k, true)] }] }));
        s.db.close(); localStorage.setItem('odmd.studio.last', h.id); }""")
    page.reload()
    page.wait_for_selector(S_READY, timeout=30000)
    page.mouse.click(*s_at(page, 40, 10))
    s_kept(page)
    r = page.evaluate(S_REC)
    with page.expect_download(timeout=60000) as dl:
        page.click("#s-gif")
    im = Image.open(io.BytesIO(Path(dl.value.path()).read_bytes()))
    check(r["saved"] == "kept" and "keys" not in r["body"] and r["on"] == {"6,0": 1, "40,10": 1} and im.size == (128, 32),
          f"/studio/ review: a key with no colour (300) in a two-byte piece of one colour opens as off, saves one byte a pixel and downloads ({r['on']}, keys {r['body'].get('keys')})")
    # Not back through the cache of read cels: a snapshot restored and undone, then a new colour (the lowest free key);
    # and a snapshot whose own cel has a key with no colour.
    o = page.evaluate("""async () => { const m = await import('/studio/store.js'), { Doc, N } = await import('/studio/doc.js');
        const a = new Uint8Array(N); a[5] = 2; a[6] = 1;
        const body = { w: 128, h: 32, colours: [{ k: 1, hex: '#102030' }], frames: [{ key: '0000000a', ms: 100 }], layers: [{ key: '0000000b', name: 'L', moves: false, cels: [await m.packPx(a)] }] };
        const blank = { ...body, layers: [{ ...body.layers[0], cels: [await m.packPx(new Uint8Array(N))] }] };
        const d = await Doc.open({ hash: 'h', body }, (b) => b.colours, null);
        const opened = d.px[5];
        await d.restore(blank); const w = d.step(false); if (w) await w;
        const undone = d.px[5];
        const k = d.pickColour(0xff0000); d.begin(); d.set(100, k); d.end();
        const d2 = await Doc.open({ hash: 'h', body: blank }, (b) => b.colours, null);
        await d2.restore(body);
        return { opened, undone, newKey: k, after: d.px[5], restored: d2.px[5] }; }""")
    check(o["opened"] == 0 and o["undone"] == 0 and o["after"] == 0 and o["restored"] == 0,
          f"/studio/ review: a key with no colour stays off through a restore and its undo, a new colour after it, and a restored snapshot of its own {o}")
    page.context.close()


def studio_lost_steps(browser, base: str, site: Path, errors: list[str]) -> None:
    """Undo, new strokes, and the tab killed inside the autosave's wait: the
    reopened draft is what was stored, and redo makes no picture that never was."""
    ctx = browser.new_context(viewport={"width": 1280, "height": 800})
    page = ctx.new_page()
    page.on("pageerror", lambda e: errors.append(f"[studio] pageerror: {e}"))
    page.add_init_script("document.addEventListener('visibilitychange', (e) => e.stopImmediatePropagation(), true)")
    page.goto(f"{base}/studio/")
    page.wait_for_selector(S_READY, timeout=30000)
    s_blur(page)
    for x in (10, 20, 30):
        page.mouse.click(*s_at(page, x, 5))
    s_kept(page)
    stored = set(page.evaluate(S_REC)["on"])
    page.keyboard.press("Control+z")
    page.keyboard.press("Control+z")
    page.mouse.click(*s_at(page, 50, 20))
    page.mouse.click(*s_at(page, 60, 20))
    page.wait_for_timeout(120)  # the journal written, the save (400 ms on) not yet
    j = page.evaluate(S_REC)["j"]
    page.close()
    page = ctx.new_page()
    page.goto(f"{base}/studio/")
    page.wait_for_selector(S_READY, timeout=30000)
    page.wait_for_timeout(500)
    s_blur(page)
    opened = set(s_canvas(page))
    seen = [opened]
    for k in ("Control+z", "Control+z", "Control+z", "Control+y", "Control+y", "Control+y"):
        page.keyboard.press(k)
        page.wait_for_timeout(100)
        seen.append(set(s_canvas(page)))
    lived = [{"10,5", "20,5", "30,5"}, {"10,5", "20,5"}, {"10,5"}, set(), {"10,5", "50,20"}, {"10,5", "50,20", "60,20"}]
    check(j["hash"] is None and opened == stored and all(v in lived for v in seen),
          f"/studio/ review: steps written below where the stored record stands mark its journal not that record's; reopened, the draft is as stored and undo and redo show only pictures that were ({j}, {[sorted(v) for v in seen]})")
    ctx.close()


def studio_garbage_drafts(browser, base: str, site: Path, errors: list[str]) -> None:
    """Records in the store that are not records (written past check()): the
    drafts list draws, the Studio starts, markup in a name stays text."""
    page = s_page(browser, base, site, errors)
    page.evaluate("""async () => { const m = await import('/studio/store.js'); const s = await m.LocalStore.open();
        const z = await m.packPx(new Uint8Array(4096));
        const body = () => ({ w: 128, h: 32, colours: [{ k: 1, hex: '#f7a531' }], frames: [{ key: '0000000a', ms: 5000 }], layers: [{ key: '0000000b', name: 'D', moves: false, cels: [z] }] });
        const env = (o) => ({ ...m.record('piece', 'G', body()), hash: 'sha256-' + '0'.repeat(64), seq: 1, ...o });
        const recs = [env({ body: null }), env({ body: { frames: 5, layers: 'x' } }), env({ name: { toString: 5 }, modified: {} }), env({ name: undefined, tags: undefined, modified: undefined }),
          env({ body: { ...body(), colours: 'red' } }), env({ modified: '9999-99-99', name: '<img src=x onerror="window.__xss=1">' })];
        await new Promise((ok) => { const tx = s.db.transaction('records', 'readwrite'); for (const r of recs) tx.objectStore('records').put(r); tx.oncomplete = ok; });
        s.db.close(); localStorage.setItem('odmd.studio.last', recs[2].id); }""")
    page.reload()
    page.wait_for_selector(S_READY, timeout=30000)
    page.wait_for_timeout(300)
    n = page.eval_on_selector_all("#s-list li", "els => els.length")
    page.mouse.click(*s_at(page, 4, 4))
    s_kept(page)
    check(n >= 6 and not page.evaluate("window.__xss || 0") and s_data(page)["saved"] == "kept",
          f"/studio/ review: records that are not records are listed (never trusted), the Studio starts and saves, a name's markup stays text ({n} listed)")
    page.context.close()


def studio_starter_times(site: Path) -> None:
    """Every starter template's frames last 20 to 60,000 ms (a still 5,000), as check() asks of a piece."""
    pack = json.loads((site / "studio/starter.dmdpack").read_text())
    times = {r["name"]: [f["ms"] for f in r["body"]["frames"]] for r in pack["records"] if r["type"] == "template"}
    check(times and all(20 <= ms <= 60000 for t in times.values() for ms in t),
          f"/studio/ review: the starter templates' frame times are 20 to 60,000 ms {times}")


# --- the Studio, S2 branch 2 (studio-s2-frames): frames and layers, Play, Send to panel, the phone's tabs ---
def s_frame(page: Page) -> str:
    return s_data(page).get("frame", "")


def s_cell(page: Page, f: int) -> None:
    """Frame f (from 0) shown, by its cell in the strip."""
    page.click(f"#s-cells [data-f='{f}']")
    wait_js(page, f"document.getElementById('studio').dataset.frame?.startsWith('{f + 1}/')")


def s_layer_btn(page: Page, name: str, act: str) -> None:
    """A layer row's button (`act`: hide, moves, up, down, dupl, dell), the row found by its name."""
    page.evaluate("""([name, act]) => { const li = [...document.querySelectorAll('#s-lrows li')].find((l) => l.querySelector('.s-lname').value === name);
        li.querySelector(`[data-la="${act}"]`).click(); }""", [name, act])


def s_wait_steps(page: Page, want: str) -> None:
    wait_js(page, f"document.getElementById('studio').dataset.steps === '{want}'", timeout=10000)


def studio_frames(browser, base: str, site: Path, errors: list[str]) -> None:
    """Frames and layers in the page: frames.js only when wanted; a frame
    added, drawn on and played; a held layer on every frame, a moving one
    not; layers reordered and undone; every structure change one undo step,
    kept over a reload."""
    page = s_page(browser, base, site, errors)
    s_blur(page)
    page.mouse.click(*s_at(page, 1, 1))
    s_kept(page)
    still = s_paths(page)
    check("/studio/frames.js" not in still and "/studio/frames.css" not in still and page.locator("#s-cells .s-cell").count() == 1,
          "/studio/ frames: a one-layer still shows its one cell and +, and fetches no frames.js")
    page.click("#s-addf")
    wait_js(page, "document.getElementById('studio').dataset.frame === '2/2'")
    check("/studio/frames.js" in s_paths(page) and "/studio/frames.css" in s_paths(page) and s_steps(page) == "2/2" and not s_on(page),
          f"/studio/ frames: + fetches frames.js, adds a blank frame after this one and shows it, one step ({s_steps(page)})")
    page.mouse.click(*s_at(page, 5, 5))
    page.fill("#s-ms", "100")
    page.click("#s-msall")
    s_wait_steps(page, "5/5")
    s_kept(page)
    r = page.evaluate(S_STORE, ["get", None])["r"]["body"]
    check([f["ms"] for f in r["frames"]] == [100, 100] and r["layers"][0]["moves"] and len(r["layers"][0]["cels"]) == 2
          and page.inner_text("#s-kind").startswith("LOOP · 2 FRAMES · 0.2 S"),
          f"/studio/ frames: the frame time for every frame; the layer drawn on now moves; the kind line says so ({page.inner_text('#s-kind')})")
    page.click("#s-playb")
    seen = set()
    for _ in range(25):
        seen.add(frozenset(s_on(page)))
        page.wait_for_timeout(30)
    playing = s_data(page).get("playing")
    page.click("#s-playb")
    check(frozenset({(1, 1)}) in seen and frozenset({(5, 5)}) in seen and playing == "yes" and not s_data(page).get("playing"),
          f"/studio/ frames: Play shows the frames in turn, and stops ({[sorted(x) for x in seen]})")
    # Onion skin: the frame before, faint, under this one, where this one is off.
    s_cell(page, 1)
    page.click("#s-onionb")
    page.wait_for_timeout(200)
    skin = page.evaluate("""() => { const c = document.getElementById('s-onion'), d = c.getContext('2d').getImageData(0, 0, 128, 32).data, on = [];
        for (let i = 0; i < 4096; i++) if (d[i * 4] + d[i * 4 + 1] + d[i * 4 + 2]) on.push([i % 128, (i / 128) | 0]); return [!c.hidden, on]; }""")
    onion_px = """() => { const c = document.getElementById('s-onion'), d = c.getContext('2d').getImageData(0, 0, 128, 32).data, on = [];
        for (let i = 0; i < 4096; i++) if (d[i * 4] + d[i * 4 + 1] + d[i * 4 + 2]) on.push([i % 128, (i / 128) | 0]); return on; }"""
    page.mouse.click(*s_at(page, 1, 1))  # this frame lit where the one before is: the skin is not drawn there
    page.wait_for_timeout(200)
    covered = page.evaluate(onion_px)
    page.keyboard.press("Control+z")
    page.click("#s-onionb")
    page.wait_for_timeout(100)
    check(skin[0] and skin[1] == [[1, 1]] and covered == [] and page.is_hidden("#s-onion") and s_on(page) == {(5, 5)},
          f"/studio/ frames: onion skin shows the frame before, faint, only where this one is off, and goes ({skin}, {covered})")
    # A held layer, drawn once, shows on every frame; the moving one only on its own.
    s_cell(page, 1)
    page.click("#s-addl")
    wait_js(page, "(document.getElementById('studio').dataset.layers || '').split('|').length === 2")
    page.click('#s-set .s-sw[data-hex="#33d1ff"]')
    page.mouse.click(*s_at(page, 10, 10))
    s_kept(page)
    s_cell(page, 0)
    one = s_on(page)
    s_cell(page, 1)
    two = s_on(page)
    check(one == {(1, 1), (10, 10)} and two == {(5, 5), (10, 10)},
          f"/studio/ frames: a held layer drawn once shows on every frame; a moving one does not ({sorted(one)}, {sorted(two)})")
    # Reordered: the composite changes; undone, back.
    page.mouse.click(*s_at(page, 5, 5))
    s_kept(page)
    top = s_px(page, 5, 5)
    s_layer_btn(page, "Layer 2", "down")
    wait_js(page, "document.getElementById('studio').dataset.layers === 'Layer 2|Drawing'")
    under = s_px(page, 5, 5)
    page.keyboard.press("Control+z")
    wait_js(page, "document.getElementById('studio').dataset.layers === 'Drawing|Layer 2'")
    check(top == (0x33, 0xd1, 0xff) and under == AMBER and s_px(page, 5, 5) == top,
          f"/studio/ frames: a layer moved down shows the one now above it; undone, back ({top}, {under})")
    page.keyboard.press("Control+Shift+z")
    page.keyboard.press("Control+z")
    # Every structure change one step, kept over a reload.
    acts = [("dup frame", lambda: page.click(".s-fmenu [data-fa=dup]")), ("move frame", lambda: page.click(".s-fmenu [data-fa=left]")),
            ("delete frame", lambda: page.click(".s-fmenu [data-fa=del]")), ("plays", lambda: (page.fill("#s-plays", "3"), page.press("#s-plays", "Enter"))),
            ("hide", lambda: s_layer_btn(page, "Layer 2", "hide")), ("held/moving", lambda: s_layer_btn(page, "Layer 2", "moves")),
            ("duplicate layer", lambda: s_layer_btn(page, "Layer 2", "dupl")), ("delete layer", lambda: s_layer_btn(page, "Layer 2 copy", "dell")),
            ("rename", lambda: (page.fill("#s-lrows [data-l='1'] .s-lname", "Stars"), page.press("#s-lrows [data-l='1'] .s-lname", "Enter")))]
    counts = {}
    for name, act in acts:
        n0 = int(s_steps(page).split("/")[0])
        act()
        try:
            s_wait_steps(page, f"{n0 + 1}/{n0 + 1}")
        except Exception:
            pass
        counts[name] = int(s_steps(page).split("/")[0]) - n0
    s_kept(page)
    before = page.evaluate(S_STORE, ["get", None])["r"]
    page.reload()
    page.wait_for_selector(S_READY, timeout=30000)
    wait_js(page, "document.getElementById('studio').dataset.frames === 'yes'")
    after_ = page.evaluate(S_STORE, ["get", None])["r"]
    st = s_steps(page)
    page.keyboard.press("Control+z")
    wait_js(page, "document.getElementById('studio').dataset.layers?.includes('Layer 2')")
    b = before["body"]
    k = int(st.split("/")[1])
    check(all(v == 1 for v in counts.values()) and after_["hash"] == before["hash"] and st == f"{k}/{k}" and s_steps(page) == f"{k - 1}/{k}"
          and b.get("plays") == 3 and len(b["frames"]) == 2 and [l["name"] for l in b["layers"]] == ["Drawing", "Stars"] and b["layers"][1].get("hide") is True,
          f"/studio/ frames: each structure change is one undo step {counts}; over a reload they are undone in turn ({st}, {[l['name'] for l in b['layers']]})")
    check(not page.evaluate("window.__csp || []"), "/studio/ frames: no CSP violation")
    page.context.close()


S_WHOLE = """async ([spec]) => { const m = await import('/studio/studio.js');
    const N = 4096, rnd = (s) => () => (s = (s * 1103515245 + 12345) >>> 0) >>> 8;
    const cel = (f) => { const a = spec.wide ? new Uint16Array(N) : new Uint8Array(N); for (let i = 0; i < N; i++) a[i] = f(i % 128, (i / 128) | 0, i); return a; };
    const layers = spec.layers.map((l) => ({ name: l.name, moves: l.moves, hide: l.hide, cels: (l.moves ? spec.frames : [0]).map((_, f) => cel((x, y, i) => {
      if (l.noise) { const r = rnd(i * 7919 + f * 104729 + 1)(); return 1 + (r % spec.colours); }
      return l.box && x >= l.box[0] + 10 * f * (l.moves ? 1 : 0) && x < l.box[0] + l.box[2] + 10 * f * (l.moves ? 1 : 0) && y >= l.box[1] && y < l.box[1] + l.box[3] ? l.k : 0; })) }));
    const colours = Array.from({ length: spec.colours }, (_, i) => ({ k: i + 1, hex: '#' + (((i + 1) * 2654435761) % 16777216 >>> 0).toString(16).padStart(6, '0') }));
    await m.piece({ name: spec.name, frames: spec.frames.map((ms) => ({ ms })), layers, colours, from: spec.from || [] }); }"""


def gif_frames(data: bytes) -> list[tuple[list, int]]:
    im = Image.open(io.BytesIO(data))
    out = []
    try:
        while True:
            out.append((list(im.convert("RGB").getdata()), im.info.get("duration")))
            im.seek(im.tell() + 1)
    except EOFError:
        pass
    return out


def studio_frames_out(browser, base: str, site: Path, errors: list[str]) -> None:
    """Download GIF of a 3-frame piece with a hidden layer; a 2-frame piece
    past 256 colours, exact and fitted."""
    page = s_page(browser, base, site, errors)
    page.evaluate(S_WHOLE, [{"name": "Three", "frames": [100, 200, 300], "colours": 3, "layers": [
        {"name": "Sky", "moves": False, "box": [0, 0, 128, 4], "k": 1}, {"name": "Rocket", "moves": True, "box": [10, 10, 8, 4], "k": 2},
        {"name": "Ghost", "moves": False, "hide": True, "box": [0, 28, 128, 4], "k": 3}]}])
    wait_js(page, "document.getElementById('s-name').value === 'Three' && document.getElementById('studio').dataset.frames === 'yes'")
    page.mouse.click(*s_at(page, 100, 20))
    s_kept(page)
    want = []
    for f in range(3):
        s_cell(page, f)
        want.append(s_pixels(page))
    with page.expect_download(timeout=60000) as dl:
        page.click("#s-gif")
    fr = gif_frames(Path(dl.value.path()).read_bytes())
    check(len(fr) == 3 and [d for _, d in fr] == [100, 200, 300] and all(fr[f][0] == want[f] for f in range(3))
          and not any(fr[f][0][30 * 128 + x] != (0, 0, 0) for f in range(3) for x in range(128)),
          f"/studio/ frames out: Download GIF of 3 frames: their times, each the composite, the hidden layer absent ({len(fr)} frames, {[d for _, d in fr]})")
    page.evaluate(S_WHOLE, [{"name": "Many", "frames": [500, 700], "colours": 300, "wide": True, "layers": [{"name": "Noise", "moves": True, "noise": True}]}])
    wait_js(page, "document.getElementById('s-name').value === 'Many'")
    page.mouse.click(*s_at(page, 0, 0))
    s_kept(page)
    want = []
    for f in range(2):
        s_cell(page, f)
        want.append(s_pixels(page))
    with page.expect_download(timeout=60000) as dl:
        page.click("#s-gif")
    fr = gif_frames(Path(dl.value.path()).read_bytes())
    t, ends = 0, {}
    for px, d in fr:
        t += d
        ends[t] = px
    with page.expect_download(timeout=60000) as dl:
        page.click("#s-fit")
    fit = gif_frames(Path(dl.value.path()).read_bytes())
    colours = len(set(p for px, _ in fit for p in px))
    check(len(fr) > 2 and ends.get(500) == want[0] and ends.get(1200) == want[1] and len(fit) == 2 and colours <= 256 and [d for _, d in fit] == [500, 700],
          f"/studio/ frames out: 2 frames past 256 colours: exact (each frame whole at its end, {len(fr)} GIF frames), and fitted (2 frames, {colours} colours)")
    page.context.close()


S_IMPORTGIF = ROOT / "opendmd-web/www/js/importgif.js"
# A stand-in panel page at #import: says ready (unless ?silent), and shows what it got, the GIF as its SHA-256.
SEND_STUB = ("<!doctype html><title>waiting</title><script>addEventListener('message', async (e) => { const d = e.data, g = d && d.gif;"
             "const h = g instanceof ArrayBuffer ? [...new Uint8Array(await crypto.subtle.digest('SHA-256', g))].map((x) => x.toString(16).padStart(2, '0')).join('') : String(g);"
             "document.title = JSON.stringify({origin: e.origin, data: {...d, gif: g instanceof ArrayBuffer ? {bytes: g.byteLength, head: String.fromCharCode(...new Uint8Array(g, 0, 4)), sha: h} : h}}); });"
             "if (opener && !location.hash.includes('silent') && !NOREADY) opener.postMessage({type: 'opendmd-import-ready'}, '*');</script>")


def send_stub(page: Page, port: int, ready: bool = True, redirect: str = "") -> list[str]:
    """http://127.0.0.1:<port>/ answered here (never the network): the stand-in panel."""
    seen: list[str] = []
    body = (f"<!doctype html><script>location.replace({json.dumps(redirect)})</script>" if redirect
            else SEND_STUB.replace("NOREADY", "false" if ready else "true"))
    page.context.route(f"http://127.0.0.1:{port}/**", lambda r: (seen.append(r.request.url), r.fulfill(body=body, content_type="text/html")))
    return seen


def studio_send(browser, base: str, site: Path, errors: list[str]) -> None:
    """Send to panel: the sheet, the address asked once (one off the home
    network asked twice), the panel's app at #import gets the message
    released firmware takes, the GIF equal to Download's, on its ready, from
    this origin; an import's naming fields offered and edited; a ready from
    another origin or window sends nothing; a blocked pop-up said; a GIF
    over 4 MB said before any window opens."""
    imp = S_IMPORTGIF.read_text()
    check("READY = 'opendmd-import-ready', IMPORT = 'opendmd-import'" in imp and "MAX_GIF = 4 * 1024 * 1024" in imp
          and "READY = 'opendmd-import-ready', IMPORT = 'opendmd-import', MAX_GIF = 4 * 1024 * 1024" in (ROOT / "opendmd-web/share/studio/out.js").read_text(),
          "/studio/ send: the message's names and the 4 MB cap are the web app's own (js/importgif.js)")
    page = s_page(browser, base, site, errors)
    port, other, silent = free_port(), free_port(), free_port()
    seen = send_stub(page, port)
    s_blur(page)
    page.fill("#s-name", "Night flight")
    page.mouse.click(*s_at(page, 3, 3))
    s_kept(page)
    page.click("#s-send")
    page.wait_for_selector("#s-p[open] #s-p-send:not([disabled])", timeout=60000)
    check("/studio/out.js" in s_paths(page) and "/panel.js" in s_paths(page) and page.input_value("#s-p-title") == "Night flight"
          and page.input_value("#s-p-platform") == "" and not seen, "/studio/ send: the sheet, the GIF made, titled with the piece's name, nothing opened yet")
    page.click("#s-p-send")
    check(page.is_visible("#s-p-ask") and not seen, "/studio/ send: asks for the panel's address")
    page.fill("#s-p-addr", "example.com")
    page.click("#s-p-go")
    asked = page.inner_text("#s-p-addr-err")
    check("Press Send again" in asked and not seen, f"/studio/ send: an address off the home network is asked about first ('{asked[:50]}')")
    page.fill("#s-p-addr", f"127.0.0.1:{port}")
    with page.context.expect_page() as info:
        page.click("#s-p-go")
    got = sent(info.value)
    d = got["data"]
    url = info.value.url
    info.value.close()
    page.evaluate("document.getElementById('s-p').close()")  # the sheet closed
    with page.expect_download(timeout=60000) as dl:
        page.click("#s-gif")
    import hashlib
    sha = hashlib.sha256(Path(dl.value.path()).read_bytes()).hexdigest()
    want = {"type": "opendmd-import", "name": "Night_flight.gif", "title": "Night flight", "platform": "", "author": "", "logo": False}
    check(url == f"http://127.0.0.1:{port}/#import" and got["origin"] == base and {k: d.get(k) for k in want} == want and d["gif"]["head"] == "GIF8"
          and d["gif"]["sha"] == sha and page.inner_text("#s-p-where-addr") == f"127.0.0.1:{port}",
          f"/studio/ send: the panel's app at #import gets the GIF (Download's bytes) and its fields on its ready, from this origin ({url}, {str(got)[:140]})")
    # An import's naming fields, offered and edited.
    page.evaluate(S_WHOLE, [{"name": "Quest", "frames": [120, 120], "colours": 2, "layers": [{"name": "A", "moves": True, "box": [0, 0, 8, 8], "k": 2}],
                             "from": [{"external": True, "note": "Game art: Star Quest", "title": "Star Quest", "platform": "SNES", "author": "pixelfan", "logo": True}]}])
    wait_js(page, "document.getElementById('s-name').value === 'Quest'")
    page.mouse.click(*s_at(page, 60, 20))
    s_kept(page)
    page.click("#s-send")
    page.wait_for_selector("#s-p[open] #s-p-send:not([disabled])", timeout=60000)
    offered = [page.input_value(f"#s-p-{k}") for k in ("title", "platform", "author")] + [page.is_checked("#s-p-logo")]
    page.fill("#s-p-author", "me2")
    with page.context.expect_page() as info:
        page.click("#s-p-send")
    d = sent(info.value)["data"]
    info.value.close()
    check(offered == ["Star Quest", "SNES", "pixelfan", True] and (d["title"], d["platform"], d["author"], d["logo"], d["name"]) == ("Star Quest", "SNES", "me2", True, "Star_Quest.gif"),
          f"/studio/ send: an import's title, platform, author and logo offered, and sent as edited ({offered}, {d['author']})")
    # A ready from another origin (the window went elsewhere) sends nothing.
    page.evaluate(f"localStorage.setItem('odmd.display', '127.0.0.1:{silent}')")
    other_seen = send_stub(page, other)
    send_stub(page, silent, redirect=f"http://127.0.0.1:{other}/#import")
    with page.context.expect_page() as info:
        page.click("#s-p-send")
    w = info.value
    w.wait_for_url(f"http://127.0.0.1:{other}/**", timeout=10000)
    w.wait_for_timeout(1500)
    check(w.title() == "waiting" and other_seen, "/studio/ send: a ready from another origin than the panel's gets nothing")
    w.close()
    # A ready from another window at the panel's origin gets nothing.
    page.unroute(f"http://127.0.0.1:{silent}/**")
    send_stub(page, silent, ready=False)
    with page.context.expect_page() as info:
        page.click("#s-p-send")
    quiet = info.value
    with page.context.expect_page() as info:
        page.evaluate(f"window.open('http://127.0.0.1:{silent}/#other')")
    impostor = info.value
    impostor.evaluate("opener.postMessage({type: 'opendmd-import-ready'}, '*')")
    impostor.wait_for_timeout(1500)
    check(impostor.title() == "waiting" and quiet.title() == "waiting", "/studio/ send: a ready from another window than the one opened gets nothing")
    quiet.close()
    impostor.close()
    page.evaluate("window.open = () => null")
    page.click("#s-p-send")
    check("allow pop-ups" in page.inner_text("#s-err"), f"/studio/ send: a blocked pop-up is said ({page.inner_text('#s-err')[:50]})")
    page.keyboard.press("Escape")
    check(not page.evaluate("window.__csp || []"), "/studio/ send: no CSP violation")
    page.context.close()
    # Over 4 MB: said, Send not offered, no window.
    page = s_page(browser, base, site, errors)
    big_seen = send_stub(page, port)
    page.evaluate(f"localStorage.setItem('odmd.display', '127.0.0.1:{port}')")
    page.evaluate(S_WHOLE, [{"name": "Huge", "frames": [20] * 256, "colours": 4096, "wide": True, "layers": [{"name": "Noise", "moves": True, "noise": True}]}])
    wait_js(page, "document.getElementById('s-name').value === 'Huge'", timeout=120000)
    page.mouse.click(*s_at(page, 0, 0))
    s_kept(page)
    page.click("#s-send")
    page.wait_for_selector("#s-p #s-err:not([hidden])", timeout=240000)
    err = page.inner_text("#s-err")
    check("over the panel’s 4 MB" in err and page.is_disabled("#s-p-send") and not big_seen,
          f"/studio/ send: a GIF over 4 MB is said before any window opens ('{err[:70]}')")
    page.context.close()


def studio_send_panel(browser, base: str, site: Path, errors: list[str]) -> None:
    """Send to panel, both ends: the Studio (this build, as https://opendmd.dev)
    sends to the panel's web app (this build's, on tools/mock-device.py at
    127.0.0.1): its Add GIFs opens with the GIF and the fields; Add puts the
    GIF on the card byte for byte as Download writes it."""
    card, www = ROOT / "target/demo-card", ROOT / "opendmd-web/www-dist"
    if not (card / "filelist.txt").is_file() or not (www / "pkg").is_dir():
        check(False, "studio send end to end: needs make site (target/demo-card) and make web")
        return
    with tempfile.TemporaryDirectory(prefix="site-smoke-send-") as tmp:
        overlay = Path(tmp)
        port = free_port()
        mock = subprocess.Popen(["uv", "run", "--quiet", str(ROOT / "tools/mock-device.py"), "--port", str(port), "--quiet",
                                 "--root", str(card), "--overlay", str(overlay), "--www", str(www)],
                                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        try:
            for _ in range(200):
                try:
                    urllib.request.urlopen(f"http://127.0.0.1:{port}/api/status", timeout=1).read()
                    break
                except OSError:
                    time.sleep(0.1)
            page = new_page(browser, "laptop", errors)

            def site_(route):
                u = urllib.parse.urlsplit(route.request.url)
                r = route.fetch(url=base + u.path + (f"?{u.query}" if u.query else ""))
                h = dict(r.headers)
                if u.path.startswith("/studio/") and u.path.endswith(("/", ".html")):
                    h["content-security-policy"] = csp_for(site, "/studio/*")
                route.fulfill(response=r, headers=h)
            page.context.route("https://opendmd.dev/**", site_)
            page.goto("https://opendmd.dev/studio/")
            page.wait_for_selector(S_READY, timeout=30000)
            page.evaluate(f"localStorage.setItem('odmd.display', '127.0.0.1:{port}')")
            page.evaluate(S_WHOLE, [{"name": "Panel piece", "frames": [150, 150, 150], "colours": 2, "layers": [{"name": "A", "moves": True, "box": [4, 4, 8, 8], "k": 2}],
                                     "from": [{"external": True, "note": "Game art", "title": "Panel Piece", "platform": "SNES", "author": "tester1", "logo": False}]}])
            wait_js(page, "document.getElementById('s-name').value === 'Panel piece'")
            page.mouse.click(*s_at(page, 100, 20))
            s_kept(page)
            page.click("#s-send")
            page.wait_for_selector("#s-p[open] #s-p-send:not([disabled])", timeout=60000)
            with page.context.expect_page() as info:
                page.click("#s-p-send")
            app = info.value
            app.on("dialog", lambda d: d.dismiss())
            app.wait_for_selector("#dl .dl-item", timeout=30000)
            got = app.evaluate("""() => { const i = document.querySelector('#dl .dl-item'); return {title: i.querySelector('.dl-title').value,
                platform: i.querySelector('.dl-plat').value, author: i.querySelector('.dl-author').value}; }""")
            cat = app.evaluate("[...document.querySelector('#dl .dl-cat').options].map((o) => o.value).filter(Boolean)[0]")
            app.select_option("#dl .dl-cat", cat)
            app.wait_for_function("document.querySelector('#dl .dl-item').dataset.name && !document.querySelector('#dl-ok').disabled", timeout=10000)
            name = app.get_attribute("#dl .dl-item", "data-name")
            app.click("#dl-ok")
            f = overlay / "gifs" / cat / name
            deadline = time.time() + 60
            while time.time() < deadline and not f.exists():
                time.sleep(0.2)
            page.evaluate("document.getElementById('s-p').close()")
            with page.expect_download(timeout=60000) as dl:
                page.click("#s-gif")
            gif = Path(dl.value.path()).read_bytes()
            check(got == {"title": "Panel Piece", "platform": "SNES", "author": "tester1"} and f.is_file() and f.read_bytes() == gif and len(gif_frames(gif)) == 3,
                  f"studio send end to end: the panel's Add GIFs filled in ({got}), the GIF on the card byte for byte as Download writes it ({name})")
            page.context.close()
        finally:
            os.killpg(mock.pid, signal.SIGTERM)


def studio_phone_frames(browser, base: str, site: Path, errors: list[str]) -> None:
    """On a phone: the bench as tabs, the strip scrolling sideways, a layer moved by Move up."""
    page = s_page(browser, base, site, errors, "phone")
    tabs = page.eval_on_selector_all("#s-tabs [data-tab]", "els => els.map(e => e.textContent)")
    vis = lambda: [page.is_visible(s) for s in (".s-colours", ".s-layers", ".s-stamps", ".s-drafts")]
    first = vis()
    page.click("#s-tabs [data-tab=layers]")
    second = vis()
    check(tabs == ["Colours", "Layers", "Stamps", "Piece"] and first == [True, False, False, False] and second == [False, True, False, False],
          f"/studio/ (phone): the bench is tabs {tabs}, one shown at a time ({first}, {second})")
    page.evaluate(S_WHOLE, [{"name": "Strip", "frames": [100] * 8, "colours": 2, "layers": [
        {"name": "Bottom", "moves": False, "box": [0, 0, 8, 8], "k": 1}, {"name": "Top", "moves": True, "box": [0, 0, 8, 8], "k": 2}]}])
    wait_js(page, "document.getElementById('s-name').value === 'Strip' && document.getElementById('studio').dataset.frames === 'yes'")
    sw = page.evaluate("(() => { const c = document.getElementById('s-cells'); return [c.scrollWidth > c.clientWidth, document.documentElement.scrollWidth - innerWidth]; })()")
    page.click("#s-tabs [data-tab=layers]")
    s_layer_btn(page, "Bottom", "up")
    wait_js(page, "document.getElementById('studio').dataset.layers === 'Top|Bottom'")
    check(sw[0] and sw[1] <= 0 and s_data(page)["layers"] == "Top|Bottom",
          f"/studio/ (phone): the strip scrolls sideways inside itself ({sw}); Move up moves a layer ({s_data(page)['layers']})")
    check(not page.evaluate("window.__csp || []"), "/studio/ (phone) frames: no CSP violation")
    page.context.close()


def studio_frames_red(browser, base: str, site: Path, errors: list[str]) -> None:
    """The red list for frames and layers: 256 frames x 16 layers (an
    autosave packs one cel; under 150 MB); frame 257 and layer 17 refused
    with a sentence; the only frame and the only layer kept; an undo across
    a deleted frame gives back its cels exactly; another tab's reorder while
    this one draws keeps this one's work as a copy."""
    page = s_page(browser, base, site, errors)
    page.evaluate(S_WHOLE, [{"name": "Full", "frames": [20] * 256, "colours": 2, "layers": [
        {"name": f"L{i}", "moves": True, "box": [i * 8, 0, 4, 4], "k": 1 + i % 2} for i in range(16)]}])
    wait_js(page, "document.getElementById('s-name').value === 'Full' && document.getElementById('studio').dataset.frames === 'yes'", timeout=120000)
    page.mouse.click(*s_at(page, 64, 20))
    s_kept(page)
    page.evaluate("""() => { const C = CompressionStream; window.__packs = 0; window.CompressionStream = function (f) { window.__packs++; return new C(f); }; }""")
    page.mouse.click(*s_at(page, 66, 20))
    s_kept(page)
    packs = page.evaluate("window.__packs")
    mem = page.evaluate("performance.memory ? performance.memory.usedJSHeapSize : 0")
    check(packs == 1 and mem < 150 * 1024 * 1024, f"/studio/ frames red: 256 frames x 16 layers: an autosave after a stroke packs one cel ({packs}), {mem / 1e6:.0f} MB")
    page.click("#s-addf")
    page.wait_for_timeout(500)
    f257 = page.inner_text("#s-err")
    page.click("#s-addl")
    page.wait_for_timeout(500)
    l17 = page.inner_text("#s-err")
    check("256 frames at most" in f257 and "16 layers at most" in l17 and s_frame(page).endswith("/256"),
          f"/studio/ frames red: frame 257 and layer 17 refused with a sentence ('{f257}', '{l17}')")
    page.context.close()
    page = s_page(browser, base, site, errors)
    s_blur(page)
    page.mouse.click(*s_at(page, 1, 1))
    page.click("#s-addf")
    wait_js(page, "document.getElementById('studio').dataset.frame === '2/2'")
    page.mouse.click(*s_at(page, 2, 2))
    s_kept(page)
    before = page.evaluate(S_STORE, ["get", None])["r"]["hash"]
    page.click(".s-fmenu [data-fa=del]")
    wait_js(page, "document.getElementById('studio').dataset.frame === '1/1'")
    s_kept(page)
    gone = page.evaluate(S_STORE, ["get", None])["r"]["hash"]
    page.click(".s-fmenu [data-fa=del]")
    page.wait_for_timeout(300)
    last_frame = page.inner_text("#s-err")
    s_layer_btn(page, "Drawing", "dell")
    page.wait_for_timeout(300)
    last_layer = page.inner_text("#s-err")
    page.keyboard.press("Control+z")
    wait_js(page, "document.getElementById('studio').dataset.frame?.endsWith('/2')")
    s_kept(page)
    back = page.evaluate(S_STORE, ["get", None])["r"]["hash"]
    check("one frame at least" in last_frame and "one layer at least" in last_layer and gone != before and back == before,
          f"/studio/ frames red: the only frame and the only layer are kept ('{last_frame}', '{last_layer}'); an undo across a deleted frame gives its cels back exactly")
    page.context.close()
    # Another tab reorders the layers while this one draws: this one's work is kept as a copy.
    ctx = browser.new_context(viewport={"width": 1280, "height": 800})
    a = s_page(browser, base, site, errors, ctx=ctx)
    a.evaluate(S_WHOLE, [{"name": "Two tabs", "frames": [100, 100], "colours": 2, "layers": [
        {"name": "Back", "moves": False, "box": [0, 0, 8, 8], "k": 1}, {"name": "Front", "moves": True, "box": [4, 4, 8, 8], "k": 2}]}])
    wait_js(a, "document.getElementById('s-name').value === 'Two tabs'")
    a.mouse.click(*s_at(a, 60, 20))
    s_kept(a)
    b = s_page(browser, base, site, errors, ctx=ctx)
    wait_js(b, "document.getElementById('studio').dataset.frames === 'yes'")
    s_layer_btn(b, "Back", "up")
    wait_js(b, "document.getElementById('studio').dataset.layers === 'Front|Back'")
    s_kept(b)
    a.mouse.click(*s_at(a, 70, 20))
    s_kept(a)
    items = a.evaluate(S_STORE, ["list", None])
    check(len(items) == 2 and a.input_value("#s-name") == "Two tabs (copy)" and s_data(a)["layers"] == "Back|Front",
          f"/studio/ frames red: another tab's reorder while this one draws: this one's work kept as a copy ({[i['name'] for i in items]})")
    ctx.close()


def studio_shape_journals(browser, base: str, site: Path, errors: list[str]) -> None:
    """Hostile structure steps in a journal (doc.js): each dropped at open,
    the piece as stored; a good one replayed; and random structure changes,
    draws and undos against doc.js, the journal replayed to the same piece."""
    page = s_page(browser, base, site, errors)
    r = page.evaluate("""async () => { const { Doc, N } = await import('/studio/doc.js'); const m = await import('/studio/store.js');
        const px = new Uint8Array(N); px[7] = 1;
        const z = await m.packPx(px), blank = await m.packPx(new Uint8Array(N));
        const rec = { hash: 'h', body: { w: 128, h: 32, colours: [{ k: 1, hex: '#123456' }], frames: [{ key: '0000000f', ms: 100 }],
          layers: [{ key: '000000aa', name: 'a', moves: false, cels: [z] }] } };
        const b = { plays: 0, frames: [{ key: '0000000f', ms: 100 }], layers: [{ key: '000000aa', name: 'a', moves: false }] };
        const two = { ...b, frames: [...b.frames, { key: '0000000e', ms: 100 }] };
        const lay2 = { ...b, layers: [...b.layers, { key: '000000ab', name: 'b', moves: false }] };
        const t = (a, zz = {}, bb = b) => [{ n: 1, t: { b: bb, a, z: zz } }];
        const cases = {
          'a frame of ms 0': t({ ...b, frames: [{ key: '0000000f', ms: 0 }] }), 'a frame of ms 1.5': t({ ...b, frames: [{ key: '0000000f', ms: 1.5 }] }),
          'two frames with one key': t({ ...b, frames: [b.frames[0], b.frames[0]] }), 'a layer key the same as a frame key': t({ ...b, layers: [...b.layers, { key: '0000000f', name: 'b', moves: false }] }, { '0000000f': [blank, false] }),
          '257 frames': t({ ...b, frames: Array.from({ length: 257 }, (_, i) => ({ key: (0x10000000 + i).toString(16), ms: 100 })) }),
          '17 layers': t({ ...b, layers: Array.from({ length: 17 }, (_, i) => ({ key: (0x20000000 + i).toString(16), name: 'x', moves: false })) }),
          'no frames': t({ ...b, frames: [] }), 'no layers': t({ ...b, layers: [] }), 'hide false': t({ ...b, layers: [{ ...b.layers[0], hide: false }] }),
          'moves "yes"': t({ ...b, layers: [{ ...b.layers[0], moves: 'yes' }] }), 'a layer name of 61': t({ ...b, layers: [{ ...b.layers[0], name: 'n'.repeat(61) }] }),
          'plays -1': t({ ...b, plays: -1 }), 'plays 1.5': t({ ...b, plays: 1.5 }), 'a skeleton that is null': t(null), 'layers of null': t({ ...b, layers: [null] }),
          'z not an object': t(lay2, 5), 'a z entry that is a string': t(lay2, { '000000ab': blank }), 'a z entry of the wrong width flag': t(lay2, { '000000ab': [blank, 'no'] }),
          'a new layer with no cel in z': t(lay2, {}), 'a new frame of a moving layer with no cel in z': t({ ...two, layers: [{ ...b.layers[0], moves: true }] }, { '000000aa/0000000f': [z, false] }),
          'a cel in z that does not read': t(lay2, { '000000ab': ['!!', false] }), 'a cel in z of the wrong size': t(lay2, { '000000ab': [await m.packPx(new Uint8Array(100)), false] }),
          'a cel in z that is a bomb': t(lay2, { '000000ab': [await m.packPx(new Uint8Array(4 << 20)), false] }),
          'a before that is not the piece': t(lay2, { '000000ab': [blank, false] }, { ...b, layers: [{ key: '000000ac', name: 'q', moves: false }] }),
          'a frame of ms 0 on the way, then back': [{ n: 1, t: { b, a: { ...b, frames: [{ key: '0000000f', ms: 0 }] }, z: {} } }, { n: 2, t: { b: { ...b, frames: [{ key: '0000000f', ms: 0 }] }, a: b, z: {} } }],
          '17 layers on the way, then back': [{ n: 1, t: { b, a: { ...b, layers: Array.from({ length: 17 }, (_, i) => ({ key: i ? (0x20000000 + i).toString(16) : '000000aa', name: 'x', moves: false })) },
            z: Object.fromEntries(Array.from({ length: 16 }, (_, i) => [(0x20000001 + i).toString(16), [blank, false]])) } },
            { n: 2, t: { b: { ...b, layers: Array.from({ length: 17 }, (_, i) => ({ key: i ? (0x20000000 + i).toString(16) : '000000aa', name: 'x', moves: false })) }, a: b, z: {} } }],
          'a t step with pixels too': [{ n: 1, t: { b, a: lay2, z: { '000000ab': [blank, false] } }, i: Int32Array.of(1), b: Uint16Array.of(0), a: Uint16Array.of(1) }],
          'a pixel step on a layer a lying t step made': [...t(lay2, {}), { n: 2, l: '000000ab', f: '0000000f', i: Int32Array.of(1), b: Uint16Array.of(0), a: Uint16Array.of(1) }],
        };
        const out = {};
        for (const [name, list] of Object.entries(cases)) {
          try { const d = await Doc.open(rec, undefined, { list, cur: list.length, at: 0, hash: 'h' });
            out[name] = d.said === 'dropped' && d.px[7] === 1 && d.layers.length === 1 && d.frames.length === 1 && !d.steps.length ? 'dropped' : `kept (${d.said}, ${d.layers.length} layers, ${d.frames.length} frames)`; }
          catch (e) { out[name] = 'threw ' + e.message; }
        }
        const good = [{ n: 1, t: { b, a: lay2, z: { '000000ab': [blank, false] } } }, { n: 2, l: '000000ab', f: '0000000f', i: Int32Array.of(1), b: Uint16Array.of(0), a: Uint16Array.of(1) }];
        const d = await Doc.open(rec, undefined, { list: good, cur: 2, at: 0, hash: 'h' });
        // A cel in z whose keys have no colour: read as off (a key the piece has no colour for is off).
        const o = await Doc.open(rec, undefined, { list: t(lay2, { '000000ab': [await m.packPx(Uint8Array.from({ length: N }, () => 9)), false] }), cur: 1, at: 0, hash: 'h' });
        return { out, good: [d.said, d.layers.length, d.layers[1]?.cels[0].px[1], d.px[1]], orphan: [o.said, o.layers.length, o.layers[1]?.cels[0].px.some((k) => k)] }; }""")
    kept = {k: v for k, v in r["out"].items() if v != "dropped"}
    check(not kept and r["good"] == ["moved", 2, 1, 1] and r["orphan"] == ["moved", 2, False],
          f"/studio/ frames hostile: {len(r['out'])} structure steps that lie, each dropped at open with the piece as stored; a good one replayed ({kept or 'all dropped'}, {r['good']})")
    fs = [page.evaluate("""async ([seed, ops]) => { const { Doc, N } = await import('/studio/doc.js'); const m = await import('/studio/store.js');
        let a = seed >>> 0; const ri = (n) => { a = (a + 0x6d2b79f5) >>> 0; let t = Math.imul(a ^ (a >>> 15), 1 | a); t ^= t + Math.imul(t ^ (t >>> 7), 61 | t); return ((t ^ (t >>> 14)) >>> 0) % n; };
        const k8 = (d) => { for (;;) { const k = (0x10000000 + ri(0xefffffff)).toString(16); if (!d.frames.some((f) => f.key === k) && !d.layers.some((l) => l.key === k)) return k; } };
        const body0 = { w: 128, h: 32, colours: [{ k: 1, hex: '#102030' }, { k: 2, hex: '#405060' }], frames: [{ key: '0000000a', ms: 100 }],
          layers: [{ key: '0000000b', name: 'L', moves: false, cels: [await m.packPx(new Uint8Array(N))] }] };
        const d = await Doc.open({ hash: 'h', body: body0 }, undefined, null);
        let stored = { body: body0, at: 0 }; const bad = []; let replays = 0, stale = false;
        // A step written at or below where the stored record stands: the journal no longer leads from it (store.js jot clears its hash).
        const push = d.push.bind(d); d.push = (st) => { const r = push(st); if (r.step.n <= stored.at) stale = true; return r; };
        const state = async (x) => { await x.loadAll(); return JSON.stringify([x.skel(), x.layers.map((l) => l.cels.map((c) => Array.from(c.px).join('')))]); };
        for (let op = 0; op < ops; op++) {
          try {
            const r = ri(12), s = d.skel(), made = {};
            await d.load();
            if (r < 3) { d.begin(); for (let n = 0; n < 4; n++) d.set(ri(N), 1 + ri(2)); d.end(); }
            else if (r === 3 && s.frames.length < 6) { const key = k8(d); s.frames.splice(d.F + 1, 0, { key, ms: 20 + ri(500) }); s.layers.forEach((l) => { if (l.moves) made[`${l.key}/${key}`] = new Uint16Array(N); }); await d.shape(s, made); }
            else if (r === 4 && s.frames.length > 1) { s.frames.splice(ri(s.frames.length), 1); await d.shape(s); }
            else if (r === 5 && s.layers.length < 5) { const key = k8(d); s.layers.splice(ri(s.layers.length + 1), 0, { key, name: 'n', moves: false }); made[key] = new Uint16Array(N); await d.shape(s, made); }
            else if (r === 6 && s.layers.length > 1) { s.layers.splice(ri(s.layers.length), 1); await d.shape(s); }
            else if (r === 7 && s.layers.length > 1) { const i = ri(s.layers.length - 1); [s.layers[i], s.layers[i + 1]] = [s.layers[i + 1], s.layers[i]]; await d.shape(s); }
            else if (r === 8) { const i = ri(s.layers.length), l = s.layers[i]; await d.loadAll();
              if (l.moves) made[l.key] = d.celAt(i, d.F).px.slice(); else for (const fr of s.frames) made[`${l.key}/${fr.key}`] = d.celAt(i, 0).px.slice();
              l.moves = !l.moves; await d.shape(s, made); }
            else if (r === 9) { const w = d.step(false); if (w) await w; }
            else if (r === 10) { const w = d.step(true); if (w) await w; }
            else { d.L = ri(d.layers.length); d.F = ri(d.frames.length); }
          } catch (e) { bad.push(`${op}: ${e.message}`); }
          if (op % 7 === 6 || stale) { stored = { body: await d.packed(d.snap()), at: d.low + d.cur - 1 }; stale = false; }
          if (op % 25 === 24) {
            const e = await Doc.open({ hash: 'h', body: stored.body }, undefined, { list: d.steps, cur: d.low + d.cur - 1, at: stored.at, hash: 'h' });
            if (stored.at >= d.low - 1 && (e.said === 'dropped' || await state(e) !== await state(d))) bad.push(`${op}: replay ${e.said}`);
            replays++;
            stored = { body: await d.packed(d.snap()), at: d.low + d.cur - 1 };
          }
        }
        return { bad: bad.slice(0, 4), n: bad.length, replays, frames: d.frames.length, layers: d.layers.length, steps: d.steps.length }; }""", [seed, 600]) for seed in (7, 23)]
    check(all(f["n"] == 0 and f["replays"] >= 20 for f in fs), f"/studio/ frames hostile: 2 x 600 random structure changes, draws, undos and redos against doc.js: none fails, the journal replays to the same piece {fs}")
    page.context.close()


# --- the review round on branch 2 (Fable): each reproduction a check ---
S_STORED_FRAMES = """async () => { const m = await import('/studio/store.js'); const s = await m.LocalStore.open(); const id = document.getElementById('studio').dataset.id;
    const r = await s.get(id); s.db.close(); const b = r.body, N = 4096, out = [];
    for (let f = 0; f < b.frames.length; f++) { const px = new Uint16Array(N);
      for (const l of b.layers) { if (l.hide) continue; const k = await m.unpackPx(l.cels[l.moves ? f : 0], N, b.keys === 2); for (let i = 0; i < N; i++) if (k[i]) px[i] = k[i]; }
      const on = []; px.forEach((k, i) => { if (k) on.push(i); }); out.push(on.join(',')); }
    return out; }"""


def s_shown_frames(page: Page) -> list[str]:
    out = []
    n = int(s_frame(page).split("/")[1])
    for f in range(n):
        s_cell(page, f)
        out.append(",".join(str(y * 128 + x) for x, y in sorted(s_on(page), key=lambda p: (p[1], p[0]))))
    return out


def studio_midstroke(browser, base: str, site: Path, errors: list[str]) -> None:
    """A stroke in progress (the pointer down) is never split by the strip,
    the frame actions, the layer rows or Play: each is ignored until it
    ends, and what is drawn is what is kept."""
    page = s_page(browser, base, site, errors)
    page.evaluate(S_WHOLE, [{"name": "Mid", "frames": [100, 100], "colours": 2, "layers": [
        {"name": "Back", "moves": False, "box": [0, 0, 4, 4], "k": 1}, {"name": "Front", "moves": True, "box": [60, 20, 4, 4], "k": 2}]}])
    wait_js(page, "document.getElementById('s-name').value === 'Mid' && document.getElementById('studio').dataset.frames === 'yes'")
    s_blur(page)
    results = {}
    for name, js in (("+", "document.getElementById('s-addf').click()"), ("a frame's cell", "document.querySelector('#s-cells [data-f=\"1\"]').click()"),
                     ("Delete frame", "document.querySelector('.s-fmenu [data-fa=del]').click()"), ("a layer row", "document.querySelector('#s-lrows li:last-child').click()"),
                     ("Delete layer", "document.querySelector('#s-lrows li [data-la=dell]').click()"), ("Play", "document.getElementById('s-playb').click()"),
                     ("N", None)):
        s0 = s_steps(page)
        f0, l0 = s_frame(page), s_data(page).get("layers")
        page.mouse.move(*s_at(page, 10, 10))
        page.mouse.down()
        page.mouse.move(*s_at(page, 20, 10), steps=4)
        if js:
            page.evaluate(js)
        else:
            page.keyboard.press("n")
        page.wait_for_timeout(150)
        page.mouse.move(*s_at(page, 30, 12), steps=4)
        page.mouse.up()
        s_kept(page)
        page.wait_for_timeout(200)
        n0 = int(s0.split("/")[0])
        results[name] = (s_steps(page) == f"{n0 + 1}/{n0 + 1}", s_frame(page) == f0, s_data(page).get("layers") == l0, not s_data(page).get("playing"))
        page.keyboard.press("Control+z")
        s_kept(page)
    stored, shown = page.evaluate(S_STORED_FRAMES), s_shown_frames(page)
    check(all(all(v) for v in results.values()) and stored == shown,
          f"/studio/ review: with a stroke in progress, +, a cell, Delete frame, a layer row, Delete layer, Play and N do nothing; the stroke is one step and kept as drawn {results} {stored == shown or (stored, shown)}")
    page.context.close()


def studio_wide_shape(browser, base: str, site: Path, errors: list[str]) -> None:
    """A structure change straight after the 256th colour is drawn (before the autosave)."""
    page = s_page(browser, base, site, errors)
    page.evaluate("""async () => { const m = await import('/studio/studio.js'); const N = 4096, z = () => new Uint8Array(N);
        await m.piece({ name: 'Edge', frames: [{ ms: 100 }, { ms: 100 }, { ms: 100 }], colours: Array.from({ length: 254 }, (_, i) => ({ k: i + 1, hex: '#' + (0x100000 + i * 257).toString(16) })),
          layers: [{ name: 'A', moves: true, cels: [z(), z(), z()] }] }); }""")
    wait_js(page, "document.getElementById('s-name').value === 'Edge' && document.getElementById('studio').dataset.frames === 'yes'")
    s_blur(page)
    page.click('#s-set .s-sw[data-hex="#f7a531"]')
    page.mouse.click(*s_at(page, 50, 5))
    s_kept(page)
    page.click('#s-set .s-sw[data-hex="#33d1ff"]')
    page.mouse.click(*s_at(page, 60, 5))
    page.evaluate("document.querySelector('.s-fmenu [data-fa=del]').click()")
    page.wait_for_timeout(400)
    err = page.inner_text("#s-err") if page.is_visible("#s-err") else ""
    s_kept(page)
    r = page.evaluate(S_STORE, ["get", None])["r"]["body"]
    page.keyboard.press("Control+z")
    s_kept(page)
    back = page.evaluate(S_STORE, ["get", None])["r"]["body"]
    s_cell(page, 0)
    check(not err and len(r["frames"]) == 2 and r.get("keys") == 2 and len(r["colours"]) == 256 and len(back["frames"]) == 3 and s_on(page) == {(50, 5), (60, 5)},
          f"/studio/ review: a frame deleted straight after the 256th colour is drawn: done, two bytes a pixel, and undone ('{err}', {len(r['frames'])} frames, keys {r.get('keys')})")
    page.context.close()


def studio_onion_bg(browser, base: str, site: Path, errors: list[str]) -> None:
    """Onion skin over a background: the moving layer's frame before shows,
    see-through where it is off, and the picture under it is not dimmed."""
    page = s_page(browser, base, site, errors)
    page.evaluate(S_WHOLE, [{"name": "Onion", "frames": [100, 100], "colours": 2, "layers": [
        {"name": "Sky", "moves": False, "box": [0, 0, 128, 32], "k": 1}, {"name": "Rocket", "moves": True, "box": [20, 10, 6, 4], "k": 2}]}])
    wait_js(page, "document.getElementById('s-name').value === 'Onion' && document.getElementById('studio').dataset.frames === 'yes'")
    s_cell(page, 1)
    page.click("#s-onionb")
    page.wait_for_timeout(300)
    o = page.evaluate("""() => { const c = document.getElementById('s-onion'), d = c.getContext('2d').getImageData(0, 0, 128, 32).data, on = [];
        let clear = 0; for (let i = 0; i < 4096; i++) { if (d[i * 4 + 3]) on.push(i); else clear++; } return { on, clear, op: +getComputedStyle(c).opacity }; }""")
    want = [y * 128 + x for y in range(10, 14) for x in range(20, 26)]
    check(sorted(o["on"]) == want and o["clear"] == 4096 - len(want) and 0 < o["op"] < 1,
          f"/studio/ review: onion skin over a background shows the frame before's moving layer, see-through elsewhere ({len(o['on'])} lit, {o['clear']} clear)")
    page.context.close()


def studio_send_sheet(browser, base: str, site: Path, errors: list[str]) -> None:
    """The Send sheet is modal for the keys too: N and Ctrl+Z do nothing
    behind it; Enter in a field sends and leaves it open. A port past 65535
    is refused and not kept. A long title is cut, the file name too. The
    status says the panel's page opened, not that it was sent."""
    page = s_page(browser, base, site, errors)
    port = free_port()
    send_stub(page, port)
    page.evaluate(S_WHOLE, [{"name": "T" * 110, "frames": [100, 100], "colours": 2, "layers": [{"name": "A", "moves": True, "box": [0, 0, 4, 4], "k": 2}],
                             "from": [{"external": True, "note": "x", "title": "Q" * 300}]}])
    wait_js(page, "document.getElementById('studio').dataset.frames === 'yes'")
    page.mouse.click(*s_at(page, 60, 20))
    s_kept(page)
    f0, st0 = s_frame(page), s_steps(page)
    page.click("#s-send")
    page.wait_for_selector("#s-p[open] #s-p-send:not([disabled])", timeout=60000)
    focus = page.evaluate("document.activeElement.id")
    page.focus("#s-p-send")
    for k in ("n", "Control+z", ",", "."):
        page.keyboard.press(k)
    page.wait_for_timeout(300)
    behind = (s_frame(page), s_steps(page), s_data(page).get("playing"), focus)
    page.focus("#s-p-title")
    page.keyboard.press("Enter")
    page.fill("#s-p-addr", f"127.0.0.1:99999")
    page.click("#s-p-go")
    big_port = (page.inner_text("#s-p-addr-err"), page.evaluate("localStorage.getItem('odmd.display')"))
    page.fill("#s-p-addr", f"127.0.0.1:{port}")
    with page.context.expect_page() as info:
        page.press("#s-p-addr", "Enter")
    d = sent(info.value)["data"]
    info.value.close()
    page.focus("#s-p-title")
    with page.context.expect_page() as info:
        page.keyboard.press("Enter")
    d2 = sent(info.value)["data"]
    info.value.close()
    still = page.evaluate("document.getElementById('s-p').open")
    said = page.inner_text("#s-saved")
    import re as _re
    check(behind == (f0, st0, None, "s-p-title"), f"/studio/ review: the Send sheet opens on its title; behind it, N, Ctrl+Z, , and . do nothing ({behind})")
    check("doesn’t look like an address" in big_port[0] and big_port[1] is None,
          f"/studio/ review: a port past 65535 is refused and not kept ({big_port})")
    check(still and d2["type"] == "opendmd-import" and len(d["title"]) == 80 and len(d["name"]) <= 120 and _re.fullmatch(r"[^/\\\x00-\x1f]+\.gif", d["name"], _re.I)
          and said.startswith(f"Opened 127.0.0.1:{port}"),
          f"/studio/ review: Enter in a field sends and the sheet stays; a 300-letter title cut to 80, the file name to {len(d['name'])}; the status says the page opened ('{said}')")
    page.context.close()


def studio_journal_z(browser, base: str, site: Path, errors: list[str]) -> None:
    """A structure journal holding a cel that does not read, for a frame not in view: dropped at open."""
    page = s_page(browser, base, site, errors)
    r = page.evaluate("""async () => { const { Doc, N } = await import('/studio/doc.js'); const m = await import('/studio/store.js');
        const z = await m.packPx(new Uint8Array(N));
        const rec = { hash: 'h', body: { w: 128, h: 32, colours: [{ k: 1, hex: '#123456' }], frames: [{ key: '0000000f', ms: 100 }],
          layers: [{ key: '000000aa', name: 'a', moves: true, cels: [z] }] } };
        const b = { plays: 0, frames: [{ key: '0000000f', ms: 100 }], layers: [{ key: '000000aa', name: 'a', moves: true }] };
        const a = { ...b, frames: [...b.frames, { key: '0000000e', ms: 100 }] };
        const out = {};
        for (const [name, cel] of [['does not read', '!!'], ['is a bomb', await m.packPx(new Uint8Array(4 << 20))], ['is the wrong size', await m.packPx(new Uint8Array(9))], ['is fine', z]]) {
          const d = await Doc.open(rec, undefined, { list: [{ n: 1, t: { b, a, z: { '000000aa/0000000e': [cel, false] } } }], cur: 1, at: 0, hash: 'h' });
          out[name] = [d.said, d.frames.length, d.F];
        }
        return out; }""")
    check(all(v[0] == "dropped" and v[1] == 1 for k, v in r.items() if k != "is fine") and r["is fine"][:2] == ["moved", 2] and r["is fine"][2] == 0,
          f"/studio/ review: a structure step's cel for a frame not in view that does not read, is a bomb or the wrong size: the journal dropped at open {r}")
    page.context.close()


def studio_small_things(browser, base: str, site: Path, errors: list[str]) -> None:
    """Frame times held to tens of ms (a GIF counts hundredths); a paste
    stops Play, and Play waits while a paste floats; plays N written as a
    loop count of N - 1 (N plays where it is kept), the panel looping by its
    playlist."""
    page = s_page(browser, base, site, errors)
    page.evaluate(S_WHOLE, [{"name": "Small", "frames": [100, 100], "colours": 2, "layers": [{"name": "A", "moves": True, "box": [0, 0, 4, 4], "k": 2}]}])
    wait_js(page, "document.getElementById('studio').dataset.frames === 'yes'")
    s_blur(page)
    page.fill("#s-ms", "125")
    page.press("#s-ms", "Enter")
    page.wait_for_timeout(300)
    s_kept(page)
    ms = page.evaluate(S_STORE, ["get", None])["r"]["body"]["frames"][0]["ms"]
    s_blur(page)
    page.keyboard.press("m")
    s_drag(page, (0, 0), (3, 3))
    page.keyboard.press("Control+c")
    page.keyboard.press("Enter")
    page.click("#s-playb")
    page.wait_for_timeout(150)
    was = s_data(page).get("playing")
    page.keyboard.press("Control+v")
    page.wait_for_timeout(150)
    stopped = s_data(page).get("playing")
    page.click("#s-playb")
    page.wait_for_timeout(150)
    refused = s_data(page).get("playing")
    page.keyboard.press("Enter")
    loops = {}
    for plays in (0, 1, 3):
        page.fill("#s-plays", str(plays))
        page.press("#s-plays", "Enter")
        page.wait_for_timeout(300)
        with page.expect_download(timeout=60000) as dl:
            page.click("#s-gif")
        g = Path(dl.value.path()).read_bytes()
        i = g.find(b"NETSCAPE2.0")
        loops[plays] = None if i < 0 else int.from_bytes(g[i + 13:i + 15], "little")
    check(ms == 130 and was == "yes" and not stopped and not refused and loops == {0: 0, 1: None, 3: 2},
          f"/studio/ review: 125 ms kept as 130; a paste stops Play, and Play waits while it floats ({was}, {stopped}, {refused}); plays 0, 1, 3 written as loop counts {loops}")
    page.context.close()


def studio_layout(browser, base: str, site: Path, errors: list[str]) -> None:
    """At 390 px: the frames right under the panel, the frame's settings
    folded behind Frame…, every control at least 40 px, the name the whole
    width with the kind line under it, the logo box on one line. At 1280
    px: Send to panel on the name's row."""
    page = s_page(browser, base, site, errors, "phone")
    page.evaluate(S_WHOLE, [{"name": "Night flight", "frames": [120] * 4, "colours": 2, "layers": [{"name": "A", "moves": True, "box": [0, 0, 4, 4], "k": 2}]}])
    wait_js(page, "document.getElementById('studio').dataset.frames === 'yes'")
    m = page.evaluate("""() => { const r = (q) => document.querySelector(q).getBoundingClientRect();
        const small = [...document.querySelectorAll('#studio button, #studio input, #studio select')].filter((e) => e.offsetParent && !e.classList.contains('s-sw')
          && getComputedStyle(e).visibility !== 'hidden' && e.getBoundingClientRect().height < 40).map((e) => (e.id || e.textContent || e.className).trim().slice(0, 12));
        return { stage: r('.s-stage').bottom, cells: r('#s-cells').top, tray: r('.s-tray').top, ms: !!document.getElementById('s-ms').offsetParent,
          name: r('#s-name').width, kindBelow: r('#s-kind').top >= r('#s-name').bottom - 2, small, wide: document.documentElement.scrollWidth - innerWidth }; }""")
    page.click("#s-fmore-b")
    shown = page.is_visible("#s-ms") and page.is_visible(".s-fmenu [data-fa=del]")
    page.click("#s-send")
    page.wait_for_selector("#s-p[open] #s-p-send:not([disabled])", timeout=60000)
    chk = page.evaluate("""() => { const l = document.querySelector('#s-p .s-chk').getBoundingClientRect(), b = document.getElementById('s-p-logo').getBoundingClientRect();
        return l.height >= 40 && l.height < 60 && b.top >= l.top && b.bottom <= l.bottom; }""")
    check(m["stage"] <= m["cells"] < m["tray"] and not m["ms"] and shown and m["name"] >= 300 and m["kindBelow"] and not m["small"] and m["wide"] <= 0 and chk,
          f"/studio/ review (phone): the frames under the panel, the frame's settings behind Frame…, every control 40 px or more, the name the width, the kind line under it, the logo box on one line {m}")
    page.context.close()
    page = s_page(browser, base, site, errors)
    row = page.evaluate("""() => { const a = document.getElementById('s-send').getBoundingClientRect(), n = document.getElementById('s-name').getBoundingClientRect(), g = document.getElementById('s-gif').getBoundingClientRect();
        return [Math.abs(a.top - g.top) < 4, a.top < n.bottom]; }""")
    check(all(row), f"/studio/ review (1280 px): Send to panel on the name's row, beside Download GIF ({row})")
    page.context.close()


# --- the Studio, S2 branch 3 (studio-s2-library): the Library, files, stamps, templates ---
S_ALL = """async () => { const m = await import('/studio/store.js'); const s = await m.LocalStore.open(); const out = [];
    for (const type of ['piece', 'palette', 'stamp', 'font', 'template', 'pack']) for (const h of (await s.list({ type })).items) { const r = await s.get(h.id); out.push({ id: r.id, type: r.type, rev: r.rev, hash: r.hash, name: r.name, from: r.from, author: r.author, licence: r.licence }); }
    s.db.close(); return out; }"""
STARTER_COUNTS = {"piece": 0, "stamp": 35, "palette": 17, "font": 2, "template": 4, "pack": 1}  # palettes: 16 and Den 16


def lib_open(page: Page) -> None:
    page.evaluate("location.hash = '#library'")
    wait_js(page, "document.getElementById('studio').dataset.view === 'library' && !!document.getElementById('studio').dataset.lib", timeout=30000)


def lib_counts(page: Page) -> dict:
    return {k: int(v) for k, v in (x.split(":") for x in s_data(page)["lib"].split())}


def lib_kind(page: Page, t: str) -> None:
    page.click(f"#s-lside [data-type={t}]")
    wait_js(page, f"document.querySelector('#s-lside .s-lt.on').dataset.type === '{t}'")


def lib_detail(page: Page, name: str) -> None:
    page.evaluate("(n) => [...document.querySelectorAll('#s-lgrid .s-lcard')].find((c) => c.querySelector('b').textContent === n).click()", name)
    page.wait_for_selector("#s-ldet:not([hidden])")


def file_save(page: Page, click, author: str | None = None, licence: str | None = None) -> dict:
    """A Save as file sheet answered (or None for as it is), the file read back."""
    with page.expect_download(timeout=60000) as dl:
        click()
        if author is not None or licence is not None or page.is_visible("#s-fs[open] #s-fs-author"):
            page.wait_for_selector("#s-fs[open] #s-fs-author", timeout=10000)
            if author is not None:
                page.fill("#s-fs-author", author)
            if licence is not None:
                page.select_option("#s-fs-licence", licence)
            page.click("#s-fs[open] button:has-text('Save')")
    return {"name": dl.value.suggested_filename, "data": json.loads(Path(dl.value.path()).read_text())}


def file_open(page: Page, data, name: str = "piece.dmdart", button: str = "#s-openf") -> str:
    """A file given to Open a file…: bytes, or an object written as JSON. What the page says: asked, refused."""
    buf = data if isinstance(data, (bytes, bytearray)) else data.encode() if isinstance(data, str) else json.dumps(data).encode()
    page.evaluate("delete document.getElementById('studio').dataset.opened")
    with page.expect_file_chooser(timeout=20000) as fc:
        page.click(button)
    fc.value.set_files({"name": name, "mimeType": "application/json", "buffer": buf})
    wait_js(page, "['asked', 'refused'].includes(document.getElementById('studio').dataset.opened)", timeout=60000)
    return s_data(page)["opened"]


def file_add(page: Page) -> int:
    page.click("#s-fs[open] button:has-text('Add')")
    wait_js(page, "(document.getElementById('studio').dataset.opened || '').startsWith('added:')", timeout=60000)
    return int(s_data(page)["opened"].split(":")[1])


def studio_library(browser, base: str, site: Path, errors: list[str]) -> None:
    """The Library: the starter pack's counts beside the user's drafts;
    search by name and by tag; library.js, files.js and stamps.js each only
    when used; a starter record read-only, its Duplicate editable; a piece
    from Scoreboard with the template's layers, its palette's colours and
    from; a palette deleted saying what uses it."""
    page = s_page(browser, base, site, errors)
    s_blur(page)
    page.mouse.click(*s_at(page, 3, 3))
    s_kept(page)
    before = s_paths(page)
    lib_open(page)
    c = lib_counts(page)
    check(all(f"/studio/{m}.js" not in before for m in ("library", "files", "stamps")) and "/studio/library.js" in s_paths(page)
          and c == {**STARTER_COUNTS, "piece": 1}, f"/studio/ library: opened only when asked, the starter pack's counts beside one draft {c}")
    lib_kind(page, "stamp")
    page.fill("#s-lq", "alien")
    page.wait_for_timeout(200)
    by_name = int(s_data(page)["libShown"])
    page.fill("#s-lq", "animated")
    page.wait_for_timeout(200)
    by_tag = int(s_data(page)["libShown"])
    page.fill("#s-lq", "")
    check(by_name >= 3 and by_tag >= 1 and by_name < 35, f"/studio/ library: search by name ({by_name} aliens) and by tag ({by_tag} animated)")
    lib_kind(page, "template")
    lib_detail(page, "Scoreboard")
    ro = page.evaluate("() => ({ name: !!document.querySelector('#s-ldet .s-lname'), del: !!document.querySelector('#s-ldet [data-la=del]'), dup: !!document.querySelector('#s-ldet [data-la=dup]') })")
    page.click("#s-ldet [data-la=dup]")
    wait_js(page, "[...document.querySelectorAll('#s-lgrid .s-lcard b')].some((b) => b.textContent === 'Scoreboard copy')")
    copy = next(r for r in page.evaluate(S_ALL) if r["name"] == "Scoreboard copy")
    check(ro == {"name": False, "del": False, "dup": True} and copy["type"] == "template" and copy["from"][0]["name"] == "Scoreboard"
          and page.is_visible("#s-ldet .s-lname"), f"/studio/ library: a starter record is read-only, and its Duplicate an editable copy made from it {ro}")
    lib_detail(page, "Scoreboard")
    page.click("#s-ldet [data-la=new]")
    wait_js(page, "document.getElementById('studio').dataset.view === 'editor' && document.getElementById('s-name').value === 'Scoreboard'", timeout=30000)
    s_kept(page)
    r = page.evaluate(S_STORE, ["get", None])["r"]
    pack = json.loads((site / "studio/starter.dmdpack").read_text())
    tp = next(x for x in pack["records"] if x["name"] == "Scoreboard")
    pal = next(x for x in pack["records"] if x["id"] == tp["body"]["palette"])
    check([l["name"] for l in r["body"]["layers"]] == [l["name"] for l in tp["body"]["layers"]] and r["from"][0]["id"] == tp["id"] and r["from"][0]["hash"] == tp["hash"]
          and [c["hex"] for c in r["body"]["colours"]] == [c["hex"] for c in pal["body"]["colours"]] and s_lit(page) > 50,
          f"/studio/ library: a new piece from Scoreboard: its layers, its palette's ({pal['name']}) colours, made from it ({len(r['body']['colours'])} colours, {s_lit(page)} lit)")
    # A palette of one's own used by a piece: deleting it says so.
    page.evaluate("""async () => { const m = await import('/studio/store.js'); const s = await m.LocalStore.open();
        const p = await s.put(m.record('palette', 'My greens', { colours: [{ k: 1, hex: '#00aa00' }], source: 'mine' }));
        const z = await m.packPx(new Uint8Array(4096));
        await s.put(m.record('piece', 'Uses greens', { w: 128, h: 32, palette: p.id, colours: [{ k: 1, hex: '#00aa00' }], frames: [{ key: '0000000a', ms: 5000 }], layers: [{ key: '0000000b', name: 'D', moves: false, cels: [z] }] }));
        s.db.close(); }""")
    lib_open(page)
    lib_kind(page, "palette")
    lib_detail(page, "My greens")
    said = []
    page.once("dialog", lambda d: (said.append(d.message), d.accept()))
    page.click("#s-ldet [data-la=del]")
    page.wait_for_timeout(500)
    check(said and "Uses greens" in said[0] and not any(x["name"] == "My greens" for x in page.evaluate(S_ALL)),
          f"/studio/ library: deleting a palette says what uses it, then deletes it ('{said[0][:60] if said else ''}')")
    check(not page.evaluate("window.__csp || []"), "/studio/ library: no CSP violation")
    page.context.close()


def studio_files(browser, base: str, site: Path, errors: list[str]) -> None:
    """Files: Save as file asks who made it (CC BY needs a name) and raises
    the rev; deleted and opened again, the piece is back as it was; the same
    file twice adds nothing; a newer rev is offered as an update; the same
    id with other pixels comes in as a copy made from it; a pack of two
    stamps and a palette round-trips; a backup into an empty browser
    restores every record."""
    page = s_page(browser, base, site, errors)
    s_blur(page)
    page.fill("#s-name", "Filed piece")
    page.mouse.click(*s_at(page, 5, 5))
    s_kept(page)
    rev0 = s_data(page)["rev"]
    page.click("#s-savef")
    page.wait_for_selector("#s-fs[open] #s-fs-author", timeout=20000)
    page.fill("#s-fs-author", "")
    page.select_option("#s-fs-licence", "CC-BY-4.0")
    page.click("#s-fs[open] button:has-text('Save')")
    asked = page.inner_text("#s-fs .s-bad")
    with page.expect_download(timeout=60000) as dl:
        page.fill("#s-fs-author", "tester1")
        page.click("#s-fs[open] button:has-text('Save')")
    f = json.loads(Path(dl.value.path()).read_text())
    fr = next(x for x in f["records"] if x["id"] == f["root"])
    s_kept(page)
    now = page.evaluate(S_STORE, ["get", None])["r"]
    check("say who made it" in asked and fr["licence"] == "CC-BY-4.0" and fr["author"]["name"] == "tester1" and fr["rev"] == int(rev0) and now["rev"] == int(rev0) + 1
          and dl.value.suggested_filename == "Filed_piece.dmdart" and f["format"] == "opendmd-studio/1",
          f"/studio/ files: Save as file asks for a name with CC BY ('{asked}'), writes the licence and author, the rev rising by one ({fr['rev']} -> {now['rev']})")
    # Deleted, then opened: back with the same id, rev and hash.
    page.once("dialog", lambda d: d.accept())
    page.click(f"#s-list .s-del[data-id='{fr['id']}']")
    page.wait_for_timeout(500)
    gone = not any(x["id"] == fr["id"] for x in page.evaluate(S_ALL))
    said = file_open(page, f)
    n = file_add(page)
    back = next((x for x in page.evaluate(S_ALL) if x["id"] == fr["id"]), None)
    check(gone and said == "asked" and n == 1 and back and (back["rev"], back["hash"]) == (fr["rev"], fr["hash"]) and page.input_value("#s-name") == "Filed piece",
          f"/studio/ files: deleted and opened again, the piece is back with its id, rev and hash, and open ({back and back['rev']})")
    # The same file again adds nothing; a newer rev is offered as an update; other pixels at the same id come in as a copy.
    said = file_open(page, f)
    rows = page.eval_on_selector_all("#s-fs li", "els => els.map((e) => e.dataset.how)")
    added = file_add(page)
    import base64
    import zlib
    newer = json.loads(json.dumps(f))
    nr = next(x for x in newer["records"] if x["id"] == f["root"])
    nr["rev"] += 5
    px = bytearray(4096)
    px[7] = 1
    nr["body"]["layers"][0]["cels"][0] = base64.b64encode(zlib.compress(bytes(px))).decode()
    nr["hash"] = s_hash(nr["body"])
    file_open(page, newer)
    rows2 = page.eval_on_selector_all("#s-fs li", "els => els.map((e) => e.dataset.how)")
    page.click("#s-fs[open] button:has-text('Cancel')")
    page.wait_for_timeout(300)
    kept = next(x for x in page.evaluate(S_ALL) if x["id"] == fr["id"])
    check((kept["rev"], kept["hash"]) == (fr["rev"], fr["hash"]) and s_data(page)["opened"] == "cancelled",
          "/studio/ files: a file shown and cancelled stores nothing")
    other = json.loads(json.dumps(newer))
    next(x for x in other["records"] if x["id"] == f["root"])["rev"] = 1
    file_open(page, other)
    rows3 = page.eval_on_selector_all("#s-fs li", "els => els.map((e) => e.dataset.how)")
    file_add(page)
    all_ = page.evaluate(S_ALL)
    cp = [x for x in all_ if x["type"] == "piece" and x["id"] != fr["id"] and x["from"] and x["from"][0].get("id") == fr["id"]]
    check(rows == ["same"] and added == 0 and rows2 == ["update"] and rows3 == ["copy"] and len(cp) == 1 and cp[0]["rev"] == 1,
          f"/studio/ files: the same file again adds nothing ({rows}); a newer rev is an update ({rows2}); other pixels at the same id come in as a copy made from it ({rows3}, {len(cp)})")
    # A pack of two stamps and a palette, saved from the Library and opened again.
    ids = page.evaluate("""async () => { const m = await import('/studio/store.js'); const s = await m.LocalStore.open();
        const px = await m.packPx(Uint8Array.of(1, 0, 1, 1));
        const p = await s.put(m.record('palette', 'Pack palette', { colours: [{ k: 1, hex: '#ff8800' }], source: 'mine' }));
        const a = await s.put(m.record('stamp', 'Pack stamp A', { w: 2, h: 2, palette: p.id, frames: [{ px }] }));
        const b = await s.put(m.record('stamp', 'Pack stamp B', { w: 2, h: 2, colours: [{ k: 1, hex: '#00ff88' }], frames: [{ px }] }));
        const k = await s.put(m.record('pack', 'My pack', { members: [p, a, b].map((x) => ({ id: x.id, rev: x.rev })) }));
        s.db.close(); return [k.id, p.id, a.id, b.id]; }""")
    lib_open(page)
    lib_kind(page, "pack")
    lib_detail(page, "My pack")
    pk = file_save(page, lambda: page.click("#s-ldet [data-la=file]"), "tester1", "CC0-1.0")
    hashes = {x["id"]: x["hash"] for x in page.evaluate(S_ALL)}
    page.evaluate("""async (ids) => { const m = await import('/studio/store.js'); const s = await m.LocalStore.open(); for (const id of ids) await s.remove(id); s.db.close(); }""", ids)
    file_open(page, pk["data"], "My_pack.dmdpack", "#s-lopen")
    file_add(page)
    after = {x["id"]: x["hash"] for x in page.evaluate(S_ALL)}
    check(pk["name"] == "My_pack.dmdpack" and len(pk["data"]["records"]) == 4 and all(after.get(i) for i in ids) and all(after[i] == hashes[i] for i in ids[1:]),
          f"/studio/ files: a pack of two stamps and a palette saved as a file, deleted, opened again: every record back ({pk['name']}, {len(pk['data']['records'])} records)")
    # Back up everything, into an empty browser.
    mine = {x["id"]: x["hash"] for x in page.evaluate(S_ALL)}
    with page.expect_download(timeout=60000) as dl:
        page.click("#s-lbackup")
    bk = Path(dl.value.path()).read_bytes()
    page.context.close()
    page = s_page(browser, base, site, errors)
    empty = len(page.evaluate(S_ALL))
    file_open(page, bk, dl.value.suggested_filename)
    file_add(page)
    got = {x["id"]: x["hash"] for x in page.evaluate(S_ALL)}
    check(empty == 0 and all(got.get(i) == h for i, h in mine.items()) and not any(x["type"] == "pack" and x["name"].startswith("Backup") for x in page.evaluate(S_ALL)),
          f"/studio/ files: Back up everything, opened in an empty browser, restores every record ({len(mine)} records, {sum(got.get(i) == h for i, h in mine.items())} back)")
    page.context.close()


def studio_stamps(browser, base: str, site: Path, errors: list[str]) -> None:
    """Stamps: a starter stamp placed lands its pixels and its colours in
    one step, the piece's from naming it once however many times it is
    placed; Save as stamp, then placed in another piece; from[] at its cap
    refuses one more stamp with a sentence."""
    page = s_page(browser, base, site, errors)
    s_blur(page)
    page.click("#s-stampsb")
    wait_js(page, "(document.getElementById('studio').dataset.stamps || '').startsWith('starter:')", timeout=30000)
    n = int(s_data(page)["stamps"].split(":")[1])
    page.click("#s-sgrid .s-sbtn[data-n='0']")
    st0 = s_steps(page)
    page.mouse.move(*s_at(page, 20, 10))
    page.wait_for_timeout(100)
    ghost = page.evaluate("!document.getElementById('s-ghost').hidden")
    page.mouse.click(*s_at(page, 20, 10))
    page.mouse.click(*s_at(page, 60, 10))
    s_kept(page)
    r = page.evaluate(S_STORE, ["get", None])["r"]
    sid = s_data(page)["stamp"]
    pack = json.loads((site / "studio/starter.dmdpack").read_text())
    sr = next(x for x in pack["records"] if x["id"] == sid)
    named = [f for f in r["from"] if f.get("id") == sid]
    check(n == 35 and ghost and s_steps(page) == "2/2" and len(named) == 1 and named[0]["hash"] == sr["hash"] and s_lit(page) > 4 and len(r["body"]["colours"]) >= 1,
          f"/studio/ stamps: a starter stamp ({sr['name']}) follows the pointer and lands twice, each one step with its colours; from names it once ({len(named)}, {s_steps(page)})")
    # Save as stamp from a selection; placed in another piece.
    page.keyboard.press("m")
    s_drag(page, (15, 5), (26, 15))
    page.click("#s-selbar [data-act=stamp]")
    wait_js(page, "!!document.getElementById('studio').dataset.savedStamp", timeout=20000)
    mine = s_data(page)["savedStamp"]
    page.keyboard.press("Enter")
    s_new(page)
    page.click("#s-tabs [data-tab=stamps]") if page.is_visible("#s-tabs") else None
    page.evaluate("(id) => document.querySelector(`#s-sgrid .s-sbtn`) && 0", mine)
    page.click("#s-sgrid .s-sbtn[data-n='0']")
    page.mouse.click(*s_at(page, 100, 20))
    s_kept(page)
    r2 = page.evaluate(S_STORE, ["get", None])["r"]
    check(s_data(page)["stamps"] == "mine:1" and any(f.get("id") == mine for f in r2["from"]) and s_lit(page) > 4,
          f"/studio/ stamps: Save as stamp, then placed in another piece ({s_data(page)['stamps']}, {s_lit(page)} lit)")
    # from[] at its cap: one more stamp refused, with a sentence.
    page.evaluate("""async () => { const m = await import('/studio/studio.js');
        await m.piece({ name: 'Full from', from: Array.from({ length: 64 }, (_, i) => ({ external: true, note: `n${i}` })) }); }""")
    wait_js(page, "document.getElementById('s-name').value === 'Full from'")
    page.keyboard.press("s")
    page.mouse.click(*s_at(page, 64, 16))
    page.wait_for_timeout(300)
    err = page.inner_text("#s-err") if page.is_visible("#s-err") else ""
    check("64 things it was made from" in err and not s_on(page), f"/studio/ stamps: from[] at its cap of 64: one more stamp refused with a sentence ('{err[:60]}')")
    check(not page.evaluate("window.__csp || []"), "/studio/ stamps: no CSP violation")
    page.context.close()


S_FILES = """async ([cases]) => { const f = await import('/studio/files.js'), m = await import('/studio/store.js');
    const s = await m.LocalStore.open(); const before = (await Promise.all(['piece','palette','stamp','font','template','pack'].map((type) => s.list({ type })))).reduce((n, x) => n + x.items.length, 0); s.db.close();
    const out = {};
    for (const [name, text] of Object.entries(cases)) {
      const t = performance.now(); let said;
      try { const r = await f.read(new File([text], name + '.dmdart')); said = 'taken ' + r.records.length; } catch (e) { said = e.message; }
      out[name] = { said, ms: Math.round(performance.now() - t) };
    }
    const s2 = await m.LocalStore.open(); const after = (await Promise.all(['piece','palette','stamp','font','template','pack'].map((type) => s2.list({ type })))).reduce((n, x) => n + x.items.length, 0); s2.db.close();
    return { out, stored: after - before }; }"""


def studio_files_hostile(browser, base: str, site: Path, errors: list[str]) -> None:
    """Files are the first input from other people: the hostile-input list.
    Each refused with a sentence, nothing stored, the page responsive."""
    page = s_page(browser, base, site, errors)
    page.click("#s-openf")  # files.js in (the chooser it opens is left)
    page.wait_for_timeout(500)
    good = page.evaluate("""async () => { const m = await import('/studio/store.js');
        const z = await m.packPx(new Uint8Array(4096));
        const r = m.record('piece', 'Good', { w: 128, h: 32, colours: [{ k: 1, hex: '#102030' }], frames: [{ key: '0000000a', ms: 100 }], layers: [{ key: '0000000b', name: 'L', moves: false, cels: [z] }] });
        r.hash = await m.hashOf(r.body); return r; }""")
    import base64
    import copy
    import zlib
    F = lambda recs, root=None: json.dumps({"format": "opendmd-studio/1", "root": root or recs[0]["id"], "records": recs})
    def mut(f):
        r = copy.deepcopy(good)
        f(r)
        return r
    def rehash(r):
        r["hash"] = s_hash(r["body"])
        return r
    bomb = base64.b64encode(zlib.compress(bytes(4 << 20))).decode()
    z64 = lambda n: base64.b64encode(zlib.compress(bytes([1]) * n)).decode()
    stamp = lambda px, **kw: {**copy.deepcopy(good), "type": "stamp", "id": "st_" + good["id"][3:], "body": {"w": 2, "h": 2, "colours": [{"k": 1, "hex": "#102030"}], "frames": [{"px": px}]}, **kw}
    pack_of = lambda pid, members, **kw: {**copy.deepcopy(good), "type": "pack", "id": pid, "name": "P", "body": {"members": members}, **kw}
    pa, pb = "pk_" + "a" * 32, "pk_" + "b" * 32
    pA = rehash(pack_of(pa, [{"id": pb, "rev": 1}]))
    pB = rehash(pack_of(pb, [{"id": pa, "rev": 1}]))
    deep = "[" * 200000 + "]" * 200000
    cases = {
        "not JSON": "this is not JSON {",
        "JSON of the wrong shape": json.dumps({"hello": "world"}),
        "the wrong format": json.dumps({"format": "opendmd-studio/2", "root": good["id"], "records": [good]}),
        "no records": json.dumps({"format": "opendmd-studio/1", "root": good["id"], "records": []}),
        "10,000 records": F([good] * 10000),
        "a record over its cap": F([mut(lambda r: r["body"]["layers"][0].__setitem__("cels", ["A" * (5 << 20)]))]),
        "a deflate bomb in a cel": F([mut(lambda r: r["body"]["layers"][0].__setitem__("cels", [bomb]))]),
        "a deflate bomb in a stamp's px": F([stamp(bomb, hash="sha256-" + "0" * 64)]),
        "a wrong hash": F([mut(lambda r: r.__setitem__("hash", "sha256-" + "0" * 64))]),
        "an id of another type's prefix": F([mut(lambda r: r.__setitem__("id", "pl" + r["id"][2:]))]),
        "two records with one id": F([good, good]),
        "a pack naming a member the file lacks": F([rehash(pack_of(pa, [{"id": good["id"], "rev": 1}]))]),
        "two packs holding each other": F([pA, pB], pa),
        "a pack holding itself": F([rehash(pack_of(pa, [{"id": pa, "rev": 1}]))]),
        "a pack of 5,000 members": F([rehash(pack_of(pa, [{"id": "pl_" + format(i, "032x"), "rev": 1} for i in range(5000)]))]),
        "a licence outside the list": F([mut(lambda r: r.__setitem__("licence", "MIT"))]),
        "a root the file lacks": F([good], "pc_" + "c" * 32),
        "JSON nested 200,000 deep": deep,
        # (each with pixels that read and a hash that is right: only check() can refuse them)
        "a stamp 0 wide": F([rehash(stamp("", body={"w": 0, "h": 2, "colours": [{"k": 1, "hex": "#102030"}], "frames": [{"px": z64(0)}]}))]),
        "a stamp 33 high": F([rehash(stamp("", body={"w": 2, "h": 33, "colours": [{"k": 1, "hex": "#102030"}], "frames": [{"px": z64(66)}]}))]),
        "a stamp of 65 frames": F([rehash(stamp("", body={"w": 2, "h": 2, "colours": [{"k": 1, "hex": "#102030"}], "frames": [{"px": z64(4)}] * 65}))]),
        "a stamp with no colours": F([rehash(stamp("", body={"w": 2, "h": 2, "frames": [{"px": z64(4)}]}))]),
        "a font atlas past a panel": F([rehash({**copy.deepcopy(good), "type": "font", "id": "ft_" + good["id"][3:], "body": {"cell": {"w": 5, "h": 7}, "line": 8, "first": 32, "count": 1, "adv": [6], "atlas": {"cols": 1, "w": 100, "h": 100, "px": z64(4)}}})]),
        "a template guide off the panel": F([rehash({**copy.deepcopy(good), "type": "template", "id": "tp_" + good["id"][3:], "body": {**copy.deepcopy(good["body"]), "guides": [{"kind": "safe", "name": "g", "x": 120, "y": 0, "w": 20, "h": 4}]}})]),
        "a font atlas not its size": F([rehash({**copy.deepcopy(good), "type": "font", "id": "ft_" + good["id"][3:], "body": {"cell": {"w": 5, "h": 7}, "line": 8, "first": 32, "count": 1, "adv": [6], "atlas": {"cols": 1, "w": 2, "h": 2, "px": z64(5)}}})]),
        "a stamp frame not its size": F([rehash(stamp("", body={"w": 2, "h": 2, "colours": [{"k": 1, "hex": "#102030"}], "frames": [{"px": z64(4)}, {"px": z64(3)}]}))]),
        "a pack naming one member twice": F([rehash(pack_of(pa, [{"id": good["id"], "rev": 1}, {"id": good["id"], "rev": 1}])), good], pa),
        "__proto__ and constructor keys": F([good]).replace('{"type"', '{"__proto__": {"polluted": 1}, "constructor": {"prototype": {"polluted": 1}}, "type"', 1),
    }
    r = page.evaluate(S_FILES, [cases])
    # Refused with one of files.js's own sentences: never a crash's words.
    taken = {k: v for k, v in r["out"].items() if v["said"].startswith("taken") or "not a record" in v["said"] or "TypeError" in v["said"]
             or "call stack" in v["said"] or not v["said"].rstrip().endswith(".")}
    slow = {k: v["ms"] for k, v in r["out"].items() if v["ms"] > 5000}
    polluted = page.evaluate("({}).polluted !== undefined")
    proto = r["out"].pop("__proto__ and constructor keys")["said"]
    taken.pop("__proto__ and constructor keys", None)
    check(not taken and r["stored"] == 0 and not slow and not polluted and proto == "taken 1" and page.evaluate("1 + 1") == 2,
          f"/studio/ files hostile: {len(cases) - 1} files that lie, each refused with a sentence, nothing stored, none slow; __proto__ and constructor keys dropped as not the envelope, nothing polluted ({ {k: v['said'][:40] for k, v in taken.items()} }, {slow}, {proto})")
    # A 50 MB file: refused before it is read.
    big = page.evaluate("""async () => { const f = await import('/studio/files.js'); const t = performance.now();
        try { await f.read(new File([new Uint8Array(50 << 20)], 'big.dmdart')); return 'taken'; } catch (e) { return [e.message, Math.round(performance.now() - t)]; } }""")
    check(isinstance(big, list) and "16 MB at most" in big[0] and big[1] < 2000, f"/studio/ files hostile: a 50 MB file refused before it is read ({big})")
    # Through the page: a refusal said, nothing asked, nothing stored; markup in a name, tag and note shown as text.
    said = file_open(page, b"not JSON at all", "x.dmdart")
    msg = page.inner_text("#s-err")
    xs = copy.deepcopy(good)
    xs["name"] = '<img src=x onerror="window.__xss=1">'
    xs["tags"] = ['<b onmouseover="window.__xss=2">t</b>']
    xs["note"] = "<script>window.__xss=3</script>"
    file_open(page, {"format": "opendmd-studio/1", "root": xs["id"], "records": [xs]})
    shown = page.inner_text("#s-fs")
    file_add(page)
    lib_open(page)
    lib_detail(page, xs["name"])
    check(said == "refused" and "isn’t a Studio file" in msg and xs["name"] in shown and not page.evaluate("window.__xss || 0")
          and page.input_value("#s-ldet .s-lname") == xs["name"], f"/studio/ files hostile: a refusal said ('{msg[:50]}'); markup in a name, tag and note shown as text, never run")
    # A file claiming a starter record's id with other content: the starter one untouched.
    pack = json.loads((site / "studio/starter.dmdpack").read_text())
    st = copy.deepcopy(next(x for x in pack["records"] if x["type"] == "stamp"))
    st["name"] = "Not the starter"
    st["body"]["frames"][0]["px"] = base64.b64encode(zlib.compress(bytes([1]) * (st["body"]["w"] * st["body"]["h"]))).decode()
    st["hash"] = s_hash(st["body"])
    file_open(page, {"format": "opendmd-studio/1", "root": st["id"], "records": [st]}, button="#s-lopen")
    how = page.eval_on_selector_all("#s-fs li", "els => els.map((e) => e.dataset.how)")
    file_add(page)
    check(how == ["starter"] and not any(x["id"] == st["id"] for x in page.evaluate(S_ALL)),
          f"/studio/ files hostile: a file claiming a starter record's id: left as it is, nothing stored ({how})")
    # A piece naming a palette the file lacks: it opens on its own colours.
    lone = copy.deepcopy(good)
    lone["body"]["palette"] = "pl_" + "d" * 32
    lone["id"] = "pc_" + "e" * 32
    lone = rehash(lone)
    file_open(page, {"format": "opendmd-studio/1", "root": lone["id"], "records": [lone]}, button="#s-lopen")
    file_add(page)
    wait_js(page, f"document.getElementById('studio').dataset.id === '{lone['id']}'", timeout=20000)
    check(page.eval_on_selector_all("#s-sw .s-sw[data-k]:not(.new)", "els => els.map((e) => e.style.background)")[1:] == ["rgb(16, 32, 48)"],
          "/studio/ files hostile: a piece naming a palette the file lacks opens on its own colours")
    page.context.close()


def studio_starter_bad(browser, base: str, site: Path, errors: list[str]) -> None:
    """A starter record that fails its hash is left out, and the rest still load."""
    page = new_page(browser, "laptop", errors)
    pack = json.loads((site / "studio/starter.dmdpack").read_text())
    next(x for x in pack["records"] if x["type"] == "stamp")["hash"] = "sha256-" + "0" * 64
    page.route("**/studio/starter.dmdpack", lambda r: r.fulfill(body=json.dumps(pack), content_type="application/json"))
    page.goto(f"{base}/studio/")
    page.wait_for_selector(S_READY, timeout=30000)
    lib_open(page)
    c = lib_counts(page)
    check(c == {**STARTER_COUNTS, "stamp": 34}, f"/studio/ starter: a starter record failing its hash is left out, the rest load {c}")
    page.context.close()


def studio_open_cost(browser, base: str, site: Path, errors: list[str]) -> None:
    """Opening a 256 x 16 piece whose journal holds four snapshot steps: each cel read once (fast)."""
    page = s_page(browser, base, site, errors)
    r = page.evaluate("""async () => { const S = await import('/studio/store.js'), { Doc, N } = await import('/studio/doc.js');
        let s = 7; const rnd = () => (s = (s * 1103515245 + 12345) >>> 0) >>> 8;
        const frames = Array.from({ length: 256 }, (_, i) => ({ key: (0x10000000 + i).toString(16), ms: 20 })), layers = [];
        for (let l = 0; l < 16; l++) { const cels = []; for (let f = 0; f < 256; f++) { const a = new Uint8Array(N); for (let i = 0; i < 20; i++) a[rnd() % N] = 1 + rnd() % 2; cels.push(await S.packPx(a)); }
          layers.push({ key: (0x20000000 + l).toString(16), name: 'L', moves: true, cels }); }
        const body = { w: 128, h: 32, colours: [{ k: 1, hex: '#102030' }, { k: 2, hex: '#405060' }], frames, layers };
        const alt = { ...body, layers: body.layers.map((l, i) => (i ? l : { ...l, cels: [body.layers[0].cels[1], ...l.cels.slice(1)] })) };
        const list = [1, 2, 3, 4].map((n) => ({ n, body: { b: n % 2 ? body : alt, a: n % 2 ? alt : body } }));
        const D = DecompressionStream; let n = 0; window.DecompressionStream = function (f) { n++; return new D(f); };
        const t = performance.now(), d = await Doc.open({ hash: 'h', body }, undefined, { list, cur: 4, at: 0, hash: 'h' });
        window.DecompressionStream = D;
        return { said: d.said, ms: Math.round(performance.now() - t), reads: n }; }""")
    check(r["said"] == "moved" and r["reads"] < 200 and r["ms"] < 1500, f"/studio/ open cost: a 256 x 16 piece with four snapshot steps opens reading each cel once {r}")
    page.context.close()


# --- the review round on branch 3 (Fable, round 5): foreign files, each reproduction a check ---
S_K = r"""
  const m = await import('/studio/store.js'); const N = 4096;
  const cel = async (dots, wide) => { const a = wide ? new Uint16Array(N) : new Uint8Array(N); for (const [x, y, k] of dots || []) a[y * 128 + x] = k; return m.packPx(a, wide); };
  const fin = async (r, want = 0) => { r.hash = await m.hashOf(r.body, want); return r; };
  const K = {
    async piece(name, dots = [[1, 1, 1]], o = {}) { const r = m.record('piece', name, { w: 128, h: 32, colours: [{ k: 1, hex: '#f7a531' }, { k: 2, hex: '#33d1ff' }], frames: [{ key: '0000000a', ms: 100 }],
      layers: [{ key: '0000000b', name: 'Drawing', moves: false, cels: [await cel(dots)] }], ...(o.body || {}) }, o.env || {}); Object.assign(r, o.set || {}); return fin(r, 4096); },
    async stamp(name, w = 4, h = 4, o = {}) { const a = new Uint8Array(o.len ?? w * h).fill(o.key ?? 1); const r = m.record('stamp', name, { w, h, colours: [{ k: 1, hex: '#ff4f8b' }], frames: [{ px: await m.packPx(a) }], ...(o.body || {}) }, o.env || {}); Object.assign(r, o.set || {}); return fin(r); },
    async palette(name, o = {}) { const r = m.record('palette', name, { colours: [{ k: 1, hex: '#102030' }, { k: 2, hex: '#405060' }], source: 'x' }); Object.assign(r, o.set || {}); return fin(r); },
    async pack(name, members, o = {}) { const r = m.record('pack', name, { members: members.map((x) => ({ id: x.id, rev: x.rev })) }, o.env || {}); Object.assign(r, o.set || {}); return fin(r); },
    file: (root, records) => JSON.stringify({ format: m.FORMAT, root: root.id, records }),
    async put(r) { const s = await m.LocalStore.open(); const h = await s.put(r); s.db.close(); return h; },
  };
"""
S_RAW = """async () => { const m = await import('/studio/store.js'); const s = await m.LocalStore.open();
  const all = await new Promise((ok) => { const q = s.db.transaction('records').objectStore('records').getAll(); q.onsuccess = () => ok(q.result); });
  const revs = await new Promise((ok) => { const q = s.db.transaction('revs').objectStore('revs').getAll(); q.onsuccess = () => ok(q.result); });
  s.db.close(); return { records: all.map((r) => ({ id: r.id, type: r.type, name: r.name, rev: r.rev, gone: !!r.gone, hash: r.hash, author: r.author, licence: r.licence, keys: Object.keys(r) })),
    revs: revs.map((r) => ({ id: r.id, rev: r.rev, label: r.label, hash: r.hash })) }; }"""


def kev(page: Page, body: str, arg=None):
    return page.evaluate("async (A) => {" + S_K + body + "}", arg)


def file_rows(page: Page) -> list[dict]:
    return page.eval_on_selector_all("#s-fs li", "els => els.map((e) => ({ how: e.dataset.how, text: e.textContent, box: e.querySelector('input[type=checkbox]')?.checked ?? null }))")


def studio_files_trust(browser, base: str, site: Path, errors: list[str]) -> None:
    """A stranger's newer version of my piece: offered unticked when its
    author differs, and when taken mine is kept first as a named snapshot
    that restores; what each row claims is shown as the file's claim, and no
    account comes in from a file; records carrying the store's own keys
    (gone, seq, ver, label) refused; a rev past 2^31 refused."""
    page = s_page(browser, base, site, errors)
    got = kev(page, """const mine = await K.piece('My rocket', [[1, 1, 1], [2, 2, 1]]); mine.author = { name: 'Me', account: null }; await K.put(mine);
        const evil = await K.piece('My rocket', [[100, 20, 2]], { set: { id: mine.id, rev: 7, author: { name: 'Mallory', account: 'acct_mallory' }, licence: 'CC0-1.0', created: mine.created } });
        const same = await K.piece('My rocket', [[50, 20, 2]], { set: { id: mine.id, rev: 8, author: { name: 'Me', account: 'acct_x' }, created: mine.created } });
        return { id: mine.id, mine: mine.hash, evil: K.file(evil, [evil]), same: K.file(same, [same]), hsame: same.hash };""")
    file_open(page, got["evil"])
    rows = file_rows(page)
    file_add(page)
    raw = page.evaluate(S_RAW)
    kept = next(r for r in raw["records"] if r["id"] == got["id"])
    check(rows[0]["how"] == "update" and rows[0]["box"] is False and "by “Mallory” · licence CC0, the file says" in rows[0]["text"] and kept["hash"] == got["mine"],
          f"/studio/ files trust: a stranger's newer version is offered unticked, its claim shown as the file's; left, mine stays ({rows[0]['text'][:90]})")
    file_open(page, got["same"])
    rows = file_rows(page)
    page.check("#s-fs input[type=checkbox]")
    file_add(page)
    raw = page.evaluate(S_RAW)
    now = next(r for r in raw["records"] if r["id"] == got["id"])
    snap = [v for v in raw["revs"] if v["id"] == got["id"]]
    check(rows[0]["box"] is False and now["hash"] == got["hsame"] and now["author"]["account"] is None and len(snap) == 1 and snap[0]["label"] == "Before the file’s version" and snap[0]["hash"] == got["mine"],
          f"/studio/ files trust: one by the same name, still offered unticked (a name proves nothing), ticked and taken: mine kept first as a named snapshot ({snap}), no account taken from the file ({now['author']})")
    page.evaluate(f"localStorage.setItem('odmd.studio.last', '{got['id']}')")
    page.reload()
    page.wait_for_selector(S_READY, timeout=30000)
    page.click("#s-snaps [data-rev]")
    wait_js(page, "document.querySelectorAll('#s-snaps li').length >= 2", timeout=20000)
    s_kept(page)
    check(s_on(page) == {(1, 1), (2, 2)}, f"/studio/ files trust: that snapshot restores my version ({sorted(s_on(page))})")
    bad = kev(page, """const out = {};
        for (const [k, v] of [['gone', true], ['seq', 999999], ['ver', 'x'], ['label', 'L']]) { const r = await K.piece('Keys ' + k, [[5, 5, 1]], { set: { [k]: v } }); out[k] = K.file(r, [r]); }
        const top = await K.piece('Top rev', [[1, 1, 1]], { set: { rev: 2 ** 31 + 1 } }); out.rev = K.file(top, [top]); return out;""")
    said = {k: (file_open(page, v), page.inner_text("#s-err"))[1] for k, v in bad.items()}
    raw = page.evaluate(S_RAW)
    check(all(("only this browser’s store writes" in v) if k != "rev" else ("(rev)" in v) for k, v in said.items()) and not any(r["gone"] for r in raw["records"])
          and not any(set(r["keys"]) - {"type", "id", "rev", "hash", "name", "tags", "note", "author", "licence", "from", "created", "modified", "body", "seq"} for r in raw["records"]),
          f"/studio/ files trust: records with gone, seq, ver or label refused, and a rev past 2^31; nothing but the envelope stored { {k: v[:50] for k, v in said.items()} }")
    # A key the envelope doesn't name (a later Studio's): dropped, the record taken.
    fut = kev(page, "const p = await K.piece('From the future', [[1, 1, 1]]); p.signature = 'ed25519:abc'; return { id: p.id, text: K.file(p, [p]) };")
    said = file_open(page, fut["text"])
    file_add(page)
    stored = next((r for r in page.evaluate(S_RAW)["records"] if r["id"] == fut["id"]), None)
    check(said == "asked" and stored and "signature" not in stored["keys"], f"/studio/ files trust: a key the envelope doesn't name is dropped and the record taken ({stored and stored['keys']})")
    # An update taken that renamed my piece and its licence: Restore gives back the whole record.
    r1 = kev(page, """const mine = await K.piece('My rocket', [[1, 1, 1], [2, 2, 1], [3, 3, 1]]); mine.author = { name: 'Me', account: null }; mine.licence = 'CC-BY-4.0'; mine.tags = ['mine']; await K.put(mine);
        const evil = await K.piece('Stolen', [[100, 20, 2]], { set: { id: mine.id, rev: 7, author: { name: 'Mallory', account: null }, licence: 'CC0-1.0', tags: ['theirs'], created: mine.created } });
        const pal = await K.palette('My palette'); await K.put(pal); const newer = await K.palette('My palette', { set: { id: pal.id, rev: 5 } }); newer.body.colours = [{ k: 1, hex: '#ff0000' }]; newer.hash = await m.hashOf(newer.body);
        return { id: mine.id, pal: pal.id, phash: pal.hash, text: K.file(evil, [evil]), ptext: K.file(newer, [newer]) };""")
    file_open(page, r1["text"])
    page.check("#s-fs input[type=checkbox]")
    file_add(page)
    page.evaluate(f"localStorage.setItem('odmd.studio.last', '{r1['id']}')")
    page.reload()
    page.wait_for_selector(S_READY, timeout=30000)
    taken_ = page.evaluate(S_STORE, ["get", None])["r"]
    page.click("#s-snaps li:has-text('Before the file’s version') [data-rev]")
    wait_js(page, "document.getElementById('s-name').value === 'My rocket'", timeout=20000)
    s_kept(page)
    back = page.evaluate(S_STORE, ["get", None])["r"]
    check(taken_["name"] == "Stolen" and (back["name"], back["author"]["name"], back["licence"], back["tags"]) == ("My rocket", "Me", "CC-BY-4.0", ["mine"]) and s_on(page) == {(1, 1), (2, 2), (3, 3)},
          f"/studio/ files trust: Restore of Before the file's version gives back the whole record: name, author, licence, tags and pixels ({back['name']}, {back['author']['name']}, {back['licence']}, {back['tags']})")
    file_open(page, r1["ptext"])
    how = [x["how"] for x in file_rows(page)]
    file_add(page)
    pal = next(r for r in page.evaluate(S_RAW)["records"] if r["id"] == r1["pal"])
    check(how == ["copy"] and pal["hash"] == r1["phash"], f"/studio/ files trust: a newer version of a palette comes in as a copy, mine untouched ({how})")
    page.context.close()


def studio_files_more(browser, base: str, site: Path, errors: list[str]) -> None:
    """A backup over a library that moved on: an older one skipped unless
    ticked, the backup pack itself not counted; a palette from a file a
    swatch choice at once; with the starter pack unreachable no file opens,
    and the Library shows the starter record; a stamp whose pixels are not
    its size refused; a stamp key with no colour placed as off, and from
    gaining the stamp only once it is placed; many small pictures capped,
    the page live while a big file is read."""
    page = s_page(browser, base, site, errors)
    ids = kev(page, """const out = {}; for (const n of ['Unchanged', 'Edited since']) { const p = await K.piece(n, [[1, 1, 1]]); await K.put(p); out[n] = p.id; } return out;""")
    lib_open(page)
    with page.expect_download(timeout=20000) as dl:
        page.click("#s-lbackup")
    data = Path(dl.value.path()).read_bytes()
    kev(page, """const s = await m.LocalStore.open(); const e = await s.get(A['Edited since']); const z = await cel([[50, 5, 2]]);
        await s.put({ ...e, body: { ...e.body, layers: [{ ...e.body.layers[0], cels: [z] }] } }, { base: e.ver }); await s.snapshot(e.id, 'mine'); s.db.close();""", ids)
    page.evaluate("location.hash = ''")
    file_open(page, data, dl.value.suggested_filename)
    rows = file_rows(page)
    summary = page.inner_text("#s-fs .s-cap")
    file_add(page)
    edited = next(r for r in page.evaluate(S_ALL) if r["id"] == ids["Edited since"])
    check(sorted(r["how"] for r in rows) == ["older", "same"] and next(r for r in rows if r["how"] == "older")["box"] is False and ": 0 new, 0 newer, 0 as copies, 1 older, 1 already here." in summary
          and edited["rev"] == 2, f"/studio/ files more: a backup over a library that moved on: the older one skipped unless ticked, the backup pack not counted ('{summary}')")
    pal = kev(page, "const p = await K.palette('From a file'); return { id: p.id, text: K.file(p, [p]) };")
    file_open(page, pal["text"])
    file_add(page)
    opts = page.eval_on_selector_all("#s-pal option", "els => els.map((e) => e.value)")
    check(pal["id"] in opts, "/studio/ files more: a palette from a file is a swatch choice at once")
    # A stamp whose pixels are not its size; a stamp key with no colour.
    st = kev(page, """const short = await K.stamp('Short px', 8, 8, { len: 10 }), orphan = await K.stamp('Key 9', 4, 4, { key: 9 });
        return { short: K.file(short, [short]), orphan: K.file(orphan, [orphan]), id: orphan.id };""")
    file_open(page, st["short"])
    short_said = page.inner_text("#s-err")
    file_open(page, st["orphan"])
    file_add(page)
    s_blur(page)
    page.click("#s-stampsb")
    wait_js(page, "(document.getElementById('studio').dataset.stamps || '').startsWith('starter:')", timeout=30000)
    page.select_option("#s-stampsx select", "mine")
    wait_js(page, "(document.getElementById('studio').dataset.stamps || '').startsWith('mine:')", timeout=20000)
    page.evaluate("(id) => [...document.querySelectorAll('#s-sgrid .s-sbtn')].find((b) => b.title === 'Key 9').click()", st["id"])
    page.mouse.click(*s_at(page, 64, 16))
    page.wait_for_timeout(600)
    r = page.evaluate(S_STORE, ["get", None])
    r = r["r"] if r else None
    from_ok = not r or not any(f.get("id") == st["id"] for f in (r["from"] or []))
    check("wrong size" in short_said and not s_on(page) and from_ok,
          f"/studio/ files more: a stamp's pixels not its size refused ('{short_said[:50]}'); one whose key has no colour places nothing, and from does not name it")
    # Many small pictures: capped before any is read.
    many = page.evaluate("""async () => { const f = await import('/studio/files.js'), m = await import('/studio/store.js');
        const junk = Array.from({ length: 40000 }, () => ({ px: 'eJxjYAAAAAQAAQ==' }));
        const r = m.record('stamp', 'Pictures', { w: 2, h: 2, colours: [{ k: 1, hex: '#ff0000' }], frames: [{ px: 'eJxjYAAAAAQAAQ==' }], extra: junk });
        const t = performance.now(); try { await f.read(new File([JSON.stringify({ format: m.FORMAT, root: r.id, records: [r] })], 'p.dmdpack')); return ['taken', 0]; } catch (e) { return [e.message, Math.round(performance.now() - t)]; } }""")
    page.context.close()
    # A big file read with the page live: 2,000 records, a word on screen, timers running.
    page = s_page(browser, base, site, errors)
    big = kev(page, """const recs = []; const a = new Uint8Array(64);
        for (let i = 0; i < 2000; i++) { a[i % 64] = 1; const r = m.record('stamp', 'Stamp ' + i, { w: 8, h: 8, colours: [{ k: 1, hex: '#ff0000' }], frames: [{ px: await m.packPx(a) }] }); r.hash = await m.hashOf(r.body); recs.push(r); }
        const pk = await K.pack('Many', recs); return K.file(pk, [pk, ...recs]);""")
    page.evaluate("window.__beats = 0; window.__said = new Set(); window.__t = setInterval(() => { __beats++; __said.add(document.getElementById('s-saved').textContent); }, 50)")
    t = time.time()
    file_open(page, big, "many.dmdpack")
    dt = time.time() - t
    beats, said = page.evaluate("[__beats, [...__said]]")
    page.click("#s-fs[open] button:has-text('Cancel')")
    check(many[0].startswith("That file holds more than 32,768 pictures") and many[1] < 3000 and beats > dt * 10 and any(x.startswith("Reading the file…") for x in said),
          f"/studio/ files more: 40,000 pictures refused before they are read ({many}); 2,000 records read with the page live ({beats} beats in {dt:.1f} s) and 'Reading the file…' on screen")
    page.context.close()
    # The starter pack unreachable: no file opens; back online, the Library shows the starter record.
    page = new_page(browser, "laptop", errors)
    pack = json.loads((site / "studio/starter.dmdpack").read_text())
    real = next(x for x in pack["records"] if x["type"] == "stamp")
    page.route("**/studio/starter.dmdpack", lambda r: r.abort())
    page.goto(f"{base}/studio/")
    page.wait_for_selector(S_READY, timeout=30000)
    fake = kev(page, "const s = await K.stamp(A.name, 4, 4, { set: { id: A.id, rev: 99 } }); return K.file(s, [s]);", {"id": real["id"], "name": real["name"]})
    said = file_open(page, fake)
    msg = page.inner_text("#s-err")
    kev(page, "await K.put(await K.stamp('Planted', 4, 4, { set: { id: A } }));", real["id"])
    page.unroute("**/studio/starter.dmdpack")
    page.reload()
    page.wait_for_selector(S_READY, timeout=30000)
    lib_open(page)
    lib_kind(page, "stamp")
    card = page.evaluate("(id) => { const c = document.querySelector(`#s-lgrid [data-id='${id}']`); return c && c.querySelector('b').textContent; }", real["id"])
    errors[:] = [e for e in errors if "starter.dmdpack" not in e]  # (the pack made unreachable on purpose)
    check(said == "refused" and "starter pack couldn’t be loaded" in msg and card == real["name"] and lib_counts(page)["stamp"] == 35,
          f"/studio/ files more: with the starter pack unreachable no file opens ('{msg[:50]}'); the Library shows the starter record for its id ({card})")
    page.context.close()


def studio_backup_limits(browser, base: str, site: Path, errors: list[str]) -> None:
    """Back up everything always writes files Open takes: nine 256 x 16
    pieces open again (their cels are not loose pictures); a Library past
    16 MB is split across files, each opening, all of it back."""
    page = s_page(browser, base, site, errors)
    kev(page, """const z = await m.packPx(new Uint8Array(4096)); const s = await m.LocalStore.open();
        for (let n = 0; n < 9; n++) { const frames = Array.from({ length: 256 }, (_, i) => ({ key: 'f' + i.toString(16).padStart(7, '0'), ms: 100 }));
          const layers = Array.from({ length: 16 }, (_, i) => ({ key: 'a' + i.toString(16).padStart(7, '0'), name: 'L', moves: true, cels: Array(256).fill(z) }));
          await s.put(m.record('piece', 'Long ' + n, { w: 128, h: 32, colours: [{ k: 1, hex: '#f7a531' }], frames, layers })); } s.db.close();""")
    lib_open(page)
    with page.expect_download(timeout=120000) as dl:
        page.click("#s-lbackup")
    data = Path(dl.value.path()).read_bytes()
    page.context.close()
    page = s_page(browser, base, site, errors)
    said = file_open(page, data, dl.value.suggested_filename)
    n = file_add(page) if said == "asked" else 0
    check(said == "asked" and n == 9, f"/studio/ backup: nine 256 x 16 pieces back up and open again ({said}, {n} added)")
    page.context.close()
    # Past 16 MB: split.
    page = s_page(browser, base, site, errors)
    kev(page, """const s = await m.LocalStore.open(); let x = 5;
        for (let n = 0; n < 5; n++) { const cels = []; for (let c = 0; c < 640; c++) { const a = new Uint8Array(4096); for (let i = 0; i < 4096; i++) { x = (Math.imul(x, 1103515245) + 12345) >>> 0; a[i] = 1 + (x >>> 24) % 255; } cels.push(await m.packPx(a)); }
          const frames = Array.from({ length: 40 }, (_, i) => ({ key: 'f' + i.toString(16).padStart(7, '0'), ms: 100 }));
          const layers = Array.from({ length: 16 }, (_, l) => ({ key: 'a' + l.toString(16).padStart(7, '0'), name: 'L', moves: true, cels: cels.slice(l * 40, l * 40 + 40) }));
          await s.put(m.record('piece', 'Noisy ' + n, { w: 128, h: 32, colours: Array.from({ length: 255 }, (_, i) => ({ k: i + 1, hex: '#' + (0x100000 + i * 4099).toString(16) })), frames, layers })); } s.db.close();""")
    lib_open(page)
    got = []
    page.on("download", lambda d: got.append(d))
    page.click("#s-lbackup")
    deadline = time.time() + 120
    while time.time() < deadline and len(got) < 2:
        page.wait_for_timeout(500)
    page.wait_for_timeout(1500)
    files = [(d.suggested_filename, Path(d.path()).read_bytes()) for d in got]
    said = page.inner_text("#s-err") if page.is_visible("#s-err") else ""
    page.context.close()
    page = s_page(browser, base, site, errors)
    added = 0
    for name, data in files:
        if file_open(page, data, name) == "asked":
            added += file_add(page)
    sizes = [len(b) for _, b in files]
    check(len(files) >= 2 and all(s_ <= 16 << 20 for s_ in sizes) and added == 5 and "in 2 files: open them together (pick all 2 at once)" in said,
          f"/studio/ backup: a Library past 16 MB is backed up in {len(files)} files ({[f'{x / 1e6:.1f} MB' for x in sizes]}), each opening; all {added} pieces back ('{said[:60]}')")
    page.context.close()


def backup_files(page: Page, wait: int = 8000) -> list[tuple[str, bytes]]:
    lib_open(page)
    got = []
    page.on("download", lambda d: got.append(d))
    page.click("#s-lbackup")
    page.wait_for_timeout(wait)
    return [(d.suggested_filename, Path(d.path()).read_bytes()) for d in got]


S_LIVE = """async () => { const m = await import('/studio/store.js'); const s = await m.LocalStore.open(); const out = {};
    for (const t of ['palette', 'stamp', 'piece', 'pack']) out[t] = (await s.list({ type: t })).items.length; s.db.close(); return out; }"""


def studio_backup_parts(browser, base: str, site: Path, errors: list[str]) -> None:
    """A backup past the member cap: each part's pack within 4,096 members,
    both parts restoring in either order; a backup holding a pack given me
    whose member I have since frozen restores; a piece in one file and its
    palette, a copy, in another (a backup's parts, and files picked
    together) point at each other in either order; the snapshot list says
    Restore brings back the name too."""
    page = s_page(browser, base, site, errors)
    kev(page, """const s = await m.LocalStore.open(); const a = new Uint8Array(64);
        for (let i = 0; i < 4200; i++) { a[i % 64] = 1; await s.put(m.record('stamp', 'S' + i, { w: 8, h: 8, colours: [{ k: 1, hex: '#ff0000' }], frames: [{ px: await m.packPx(a) }] })); }
        s.db.close();""")
    files = backup_files(page)
    page.context.close()
    members = [len(next(r for r in json.loads(b)["records"] if r["id"] == json.loads(b)["root"])["body"]["members"]) for _, b in files]
    results = []
    for order in (files, files[::-1]):
        page = s_page(browser, base, site, errors)
        with page.expect_file_chooser(timeout=20000) as fc:
            page.click("#s-openf")
        fc.value.set_files([{"name": n_, "mimeType": "application/json", "buffer": d} for n_, d in order])
        wait_js(page, "['asked', 'refused'].includes(document.getElementById('studio').dataset.opened)", timeout=120000)
        page.click("#s-fs[open] button:has-text('Add')")
        wait_js(page, "(document.getElementById('studio').dataset.opened || '').startsWith('added:')", timeout=120000)
        results.append(page.evaluate(S_LIVE)["stamp"])
        page.context.close()
    check(len(files) == 2 and max(members) <= 4096 and results == [4200, 4200],
          f"/studio/ backup parts: 4,200 stamps in {len(files)} files of {members} members, restoring in either order {results}")
    # A pack given me, a member frozen since (its rev moved on), then backed up: it restores.
    page = s_page(browser, base, site, errors)
    given = kev(page, """const st = await K.stamp('In a pack'); const p = await K.pack('Given', [st]); return K.file(p, [p, st]);""")
    file_open(page, given, "given.dmdpack")
    file_add(page)
    page.evaluate("""async () => { const m = await import('/studio/store.js'); const s = await m.LocalStore.open();
        const h = (await s.list({ type: 'stamp' })).items[0]; await s.snapshot(h.id, 'moved on'); s.db.close(); }""")
    files = backup_files(page, 3000)
    page.context.close()
    page = s_page(browser, base, site, errors)
    said = file_open(page, files[0][1], files[0][0])
    n = file_add(page) if said == "asked" else 0
    check(said == "asked" and n == 2, f"/studio/ backup parts: a backup holding a pack given me, a member of it moved on since, restores ({said}, {n})")
    page.context.close()
    # A piece and a palette across the files of one opening, either order: a reference follows a copy only from the copy's own file.
    out = []
    for case in ("across, mine here", "same file, mine here", "across, empty"):
        for first in ("palette", "piece"):
            page = s_page(browser, base, site, errors)
            f = kev(page, """const mine = await K.palette('My palette'); if (A !== 'across, empty') await K.put(mine);
                const theirs = await K.palette('My palette', { set: { id: mine.id, rev: 1 } }); theirs.body.colours = [{ k: 1, hex: '#00ff00' }]; theirs.hash = await m.hashOf(theirs.body);
                const q = await K.piece('Drawn from it', [[1, 1, 1]], { body: { palette: mine.id } });
                const pa = await K.pack('Part A', [theirs], { env: { tags: ['backup'] } }), pb = await K.pack('Part B', [q], { env: { tags: ['backup'] } });
                const both = await K.pack('Both', [theirs, q], { env: { tags: ['backup'] } }), other = await K.stamp('Beside it'), ob = await K.pack('Other', [other], { env: { tags: ['backup'] } });
                return { pal: mine.id, piece: q.id, a: K.file(pa, [pa, theirs]), b: K.file(pb, [pb, q]), both: K.file(both, [both, theirs, q]), other: K.file(ob, [ob, other]) };""", case)
            order = [("a.dmdpack", f["a"]), ("b.dmdpack", f["b"])] if case != "same file, mine here" else [("both.dmdpack", f["both"]), ("other.dmdpack", f["other"])]
            if first == "piece":
                order.reverse()
            with page.expect_file_chooser(timeout=20000) as fc:
                page.click("#s-openf")
            fc.value.set_files([{"name": n_, "mimeType": "application/json", "buffer": d.encode()} for n_, d in order])
            wait_js(page, "document.getElementById('studio').dataset.opened === 'asked'", timeout=30000)
            page.click("#s-fs[open] button:has-text('Add')")
            wait_js(page, "(document.getElementById('studio').dataset.opened || '').startsWith('added:')", timeout=30000)
            recs = page.evaluate("""async (ids) => { const m = await import('/studio/store.js'); const s = await m.LocalStore.open();
                const q = await s.get(ids[1]); const p = q && await s.get(q.body.palette); s.db.close();
                return { points: q && q.body.palette, hex: p && p.body.colours[0].hex, same: q && q.body.palette === ids[0] }; }""", [f["pal"], f["piece"]])
            want = {"across, mine here": (True, "#102030"), "same file, mine here": (False, "#00ff00"), "across, empty": (True, "#00ff00")}[case]
            out.append((case, first, (recs["same"], recs["hex"]) == want))
            page.context.close()
    check(all(ok for *_, ok in out), f"/studio/ backup parts: across files a reference keeps naming the record already here; within one file it follows the copy; into an empty browser parts link up; either order {out}")
    # The snapshot list says what Restore brings back.
    page = s_page(browser, base, site, errors)
    s_blur(page)
    page.mouse.click(*s_at(page, 1, 1))
    page.fill("#s-snapname", "One")
    page.click("#s-snap")
    wait_js(page, "document.querySelector('#s-snaps li') !== null")
    line = page.inner_text("#s-snaps li")
    check("name and all" in line, f"/studio/ backup parts: the snapshot list says Restore brings back the version, name and all ('{line[:80]}')")
    page.context.close()


def studio_read_live(browser, base: str, site: Path, errors: list[str]) -> None:
    """Pictures in places the count left out (cels under a stamp's layers)
    are counted; a file of heavy records is read with the page live."""
    page = s_page(browser, base, site, errors)
    hid = page.evaluate("""async () => { const f = await import('/studio/files.js'), m = await import('/studio/store.js');
        const r = m.record('stamp', 'Hidden', { w: 2, h: 2, colours: [{ k: 1, hex: '#ff0000' }], frames: [{ px: 'eJxjYAAAAAQAAQ==' }], layers: [{ cels: Array(40000).fill('eJxjYAAAAAQAAQ==') }] });
        try { await f.read(new File([JSON.stringify({ format: m.FORMAT, root: r.id, records: [r] })], 'h.dmdpack')); return 'taken'; } catch (e) { return e.message; } }""")
    heavy = kev(page, """const recs = []; const a = new Uint8Array(64);
        for (let i = 0; i < 40; i++) { const junk = []; for (let j = 0; j < 800; j++) { a[0] = j & 255; a[1] = j >> 8; a[2] = i; junk.push({ px: await m.packPx(a) }); }
          const r = m.record('stamp', 'Heavy ' + i, { w: 8, h: 8, colours: [{ k: 1, hex: '#ff0000' }], frames: [{ px: await m.packPx(new Uint8Array(64)) }], extra: junk }); r.hash = await m.hashOf(r.body); recs.push(r); }
        const pk = await K.pack('Heavy', recs); return K.file(pk, [pk, ...recs]);""")
    page.evaluate("window.__beats = 0; window.__t = setInterval(() => __beats++, 50)")
    t = time.time()
    file_open(page, heavy, "heavy.dmdpack")
    dt = time.time() - t
    beats = page.evaluate("__beats")
    page.click("#s-fs[open] button:has-text('Cancel')")
    check(hid.startswith("That file holds more than 32,768 pictures") and beats > dt * 10,
          f"/studio/ read live: pictures under a stamp's layers counted ('{hid[:50]}'); 40 heavy records (800 pictures each) read with the page live ({beats} beats in {dt:.1f} s)")
    page.context.close()


S_STATE = """async () => { const m = await import('/studio/store.js');
    // (meta read raw first: LocalStore.open lets old notes go, which would hide one written now)
    const raw = await new Promise((ok) => { const r = indexedDB.open('opendmd-studio'); r.onsuccess = () => { const db = r.result; const q = db.transaction('meta').objectStore('meta').getAll(); q.onsuccess = () => { db.close(); ok(q.result); }; }; });
    const s = await m.LocalStore.open(); const out = {};
    for (const t of ['palette', 'piece', 'pack']) for (const h of (await s.list({ type: t })).items) { const r = await s.get(h.id); const v = { id: r.id, palette: r.body.palette, members: r.body.members && r.body.members.map((x) => x.id), hex: r.body.colours && r.type === 'palette' ? r.body.colours[0].hex : undefined }; out[r.name] ??= v; out[r.id] = v; }
    s.db.close();
    return { recs: out, notes: raw.filter((x) => String(x.k).startsWith('n:')).length }; }"""


def pick_files(page: Page, files: list[tuple[str, str]]) -> None:
    page.evaluate("delete document.getElementById('studio').dataset.opened")
    with page.expect_file_chooser(timeout=20000) as fc:
        page.click("#s-openf")
    fc.value.set_files([{"name": n, "mimeType": "application/json", "buffer": d.encode()} for n, d in files])
    wait_js(page, "['asked', 'refused'].includes(document.getElementById('studio').dataset.opened)", timeout=60000)
    page.click("#s-fs[open] button:has-text('Add')")
    wait_js(page, "(document.getElementById('studio').dataset.opened || '').startsWith('added:')", timeout=120000)


def studio_files_attacks(browser, base: str, site: Path, errors: list[str]) -> None:
    """The repointing attacks: a stranger's file dressed as part of my
    backup, opened after I restored it, repoints nothing of mine; a record
    planted beforehand under an id a copy might take is never adopted;
    nothing is noted in the store between openings; a big Add shows its
    progress."""
    PARTS = """const P = await K.palette('My palette'); const piece = await K.piece('Drawn from it', [[1, 1, 1]], { body: { palette: P.id } }); const given = await K.pack('A pack I keep', [P]);
        const b1 = await K.pack('Backup (1 of 2)', [P], { env: { tags: ['backup'] } }), b2 = await K.pack('Backup (2 of 2)', [piece, given], { env: { tags: ['backup'] } });
        const part1 = K.file(b1, [b1, P]), part2 = K.file(b2, [b2, piece, given]);"""
    page = s_page(browser, base, site, errors)
    got = kev(page, PARTS + """const evil = await K.palette('My palette'); evil.id = P.id; evil.body.colours = [{ k: 1, hex: '#ff0000' }]; evil.hash = await m.hashOf(evil.body);
        const eb = await K.pack('Free stamps!', [evil], { env: { tags: ['backup'] } }); return { P: P.id, part1, part2, evil: K.file(eb, [eb, evil]) };""")
    pick_files(page, [("b2.dmdpack", got["part2"]), ("b1.dmdpack", got["part1"])])
    mine = page.evaluate(S_STATE)["recs"]
    pick_files(page, [("free.dmdpack", got["evil"])])
    after = page.evaluate(S_STATE)
    a = after["recs"]
    check(mine["Drawn from it"]["palette"] == got["P"] and a["Drawn from it"]["palette"] == got["P"] and a["A pack I keep"]["members"] == [got["P"]]
          and a[got["P"]]["hex"] == "#102030",
          f"/studio/ files attacks: my backup restored (parts picked together), then a stranger's file dressed as part of it: my piece and my pack still point at my palette ({a['Drawn from it']['palette'][-6:]})")
    page.context.close()
    # A record planted beforehand under an id a copy might take, then my backup over a profile that holds a different palette under its id.
    import hashlib
    page = s_page(browser, base, site, errors)
    got = kev(page, PARTS + """const other = await K.palette('Not mine'); other.id = P.id; other.body.colours = [{ k: 1, hex: '#00ff00' }]; other.hash = await m.hashOf(other.body); await K.put(other);
        return { P: P.id, part1, part2 };""")
    planted = "pl_" + hashlib.sha256(f"{got['P']}|backup:00112233445566ff".encode()).digest()[:16].hex()
    plant = kev(page, "const e = await K.palette('Planted'); e.id = A; e.body.colours = [{ k: 1, hex: '#ff0000' }]; e.hash = await m.hashOf(e.body); return K.file(e, [e]);", planted)
    pick_files(page, [("planted.dmdart", plant)])
    pick_files(page, [("b1.dmdpack", got["part1"]), ("b2.dmdpack", got["part2"])])
    st = page.evaluate(S_STATE)
    r = st["recs"]
    copy = next((v for k, v in r.items() if v["id"] not in (got["P"], planted) and v.get("hex") == "#102030"), None)
    check(copy and r["Drawn from it"]["palette"] == got["P"] and r["A pack I keep"]["members"] == [got["P"]] and planted not in (r["Drawn from it"]["palette"], *r["A pack I keep"]["members"])
          and st["notes"] == 0, f"/studio/ files attacks: a record planted under an id a copy might take is never adopted; my backup's palette comes in as a copy, and (across files) my piece and pack keep naming the record already here; nothing noted in the store")
    page.context.close()
    # A big Add: progress said, nothing noted.
    page = s_page(browser, base, site, errors)
    big = kev(page, """const recs = []; const a = new Uint8Array(64);
        for (let i = 0; i < 1500; i++) { a[i % 64] = 1; const r = m.record('stamp', 'S' + i, { w: 8, h: 8, colours: [{ k: 1, hex: '#ff0000' }], frames: [{ px: await m.packPx(a) }] }); r.hash = await m.hashOf(r.body); recs.push(r); }
        const pk = await K.pack('Many', recs); return K.file(pk, [pk, ...recs]);""")
    page.evaluate("window.__said = new Set(); window.__t = setInterval(() => __said.add(document.getElementById('s-saved').textContent), 30)")
    pick_files(page, [("many.dmdpack", big)])
    said = page.evaluate("[...__said]")
    st = page.evaluate(S_STATE)
    check(any(x.startswith("Adding…") for x in said) and st["notes"] == 0, f"/studio/ files attacks: a big Add says its progress ({[x for x in said if x.startswith('Adding')][:1]}), nothing noted in the store")
    page.context.close()


def studio_open_limits(browser, base: str, site: Path, errors: list[str]) -> None:
    """One opening's limits: a stranger's file picked with part of my backup
    turns none of my references; 64 MB and 16,640 records at most in all,
    refused with a sentence; packs naming absent records write no rows for
    them, and a file naming more than 4,096 is refused."""
    page = s_page(browser, base, site, errors)
    got = kev(page, """const P = await K.palette('My palette'); await K.put(P);
        const piece = await K.piece('Drawn from it', [[1, 1, 1]], { body: { palette: P.id } }); const given = await K.pack('A pack I keep', [P]);
        const b2 = await K.pack('Backup (2 of 2)', [piece, given], { env: { tags: ['backup'] } });
        const twin = await K.palette('My palette'); twin.id = P.id; twin.body.colours = [{ k: 1, hex: '#ff0000' }]; twin.hash = await m.hashOf(twin.body);
        const sp = await K.pack('Free stamps!', [twin], { env: { tags: ['backup'] } });
        return { P: P.id, part2: K.file(b2, [b2, piece, given]), evil: K.file(sp, [sp, twin]) };""")
    pick_files(page, [("b_2_of_2.dmdpack", got["part2"]), ("free.dmdpack", got["evil"])])
    r = page.evaluate(S_STATE)["recs"]
    check(r["Drawn from it"]["palette"] == got["P"] and r["A pack I keep"]["members"] == [got["P"]] and r[got["P"]]["hex"] == "#102030",
          "/studio/ open limits: a stranger's file picked with part of my backup turns none of my references to its copy")
    page.context.close()
    # 64 MB in all: refused before any is read.
    page = s_page(browser, base, site, errors)
    page.evaluate("delete document.getElementById('studio').dataset.opened")
    with page.expect_file_chooser(timeout=20000) as fc:
        page.click("#s-openf")
    tmp = Path(tempfile.mkdtemp(prefix="site-smoke-big-"))
    paths = []
    for i in range(5):
        (tmp / f"big{i}.dmdpack").write_bytes(b"x" * (14 << 20))
        paths.append(str(tmp / f"big{i}.dmdpack"))
    fc.value.set_files(paths)
    wait_js(page, "document.getElementById('studio').dataset.opened === 'refused'", timeout=30000)
    big = page.inner_text("#s-err")
    shutil.rmtree(tmp, ignore_errors=True)
    # 16,640 records in all: five files of 4,000.
    many = kev(page, """const recs = []; const a = new Uint8Array(64);
        for (let i = 0; i < 4000; i++) { a[i % 64] = 1; const r = m.record('stamp', 'S' + i, { w: 8, h: 8, colours: [{ k: 1, hex: '#ff0000' }], frames: [{ px: await m.packPx(a) }] }); r.hash = await m.hashOf(r.body); recs.push(r); }
        const pk = await K.pack('Many', recs); return K.file(pk, [pk, ...recs]);""")
    page.evaluate("delete document.getElementById('studio').dataset.opened")
    with page.expect_file_chooser(timeout=20000) as fc:
        page.click("#s-openf")
    fc.value.set_files([{"name": f"m{i}.dmdpack", "mimeType": "application/json", "buffer": many.encode()} for i in range(5)])
    wait_js(page, "document.getElementById('studio').dataset.opened === 'refused'", timeout=120000)
    recs = page.inner_text("#s-err")
    stored = page.evaluate(S_LIVE)
    check("64 MB at most at once" in big and "more than 16,640 records together" in recs and stored["stamp"] == 0,
          f"/studio/ open limits: 70 MB picked at once refused ('{big[:50]}'), and 20,000 records ('{recs[:50]}'); nothing stored")
    page.context.close()
    # Absent members: no rows for them; more than 4,096 refused.
    page = s_page(browser, base, site, errors)
    packs = kev(page, """const ids = Array.from({ length: 4000 }, () => ({ id: m.newId('stamp'), rev: 1 })); const pk = m.record('pack', 'Names absent ones', { members: ids }); pk.hash = await m.hashOf(pk.body);
        const st = await K.stamp('Here'); const pk2 = m.record('pack', 'Names one here', { members: [{ id: st.id, rev: st.rev }] }); pk2.hash = await m.hashOf(pk2.body);
        const root = await K.pack('Root', [st], {}); const overs = []; for (let i = 0; i < 2; i++) { const o = m.record('pack', 'Too many ' + i, { members: Array.from({ length: 2500 }, () => ({ id: m.newId('stamp'), rev: 1 })) }); o.hash = await m.hashOf(o.body); overs.push(o); }
        const r2 = await K.pack('Root 2', []);
        return { ok: K.file(root, [root, st, pk, pk2]), over: K.file(r2, [r2, ...overs]) };""")
    est0 = page.evaluate("navigator.storage.estimate().then((e) => e.usage)")
    pick_files(page, [("absent.dmdpack", packs["ok"])])
    rows = page.evaluate("""() => new Promise((ok) => { const r = indexedDB.open('opendmd-studio'); r.onsuccess = () => { const db = r.result; const q = db.transaction('refs').objectStore('refs').count(); q.onsuccess = () => { db.close(); ok(q.result); }; }; })""")
    est1 = page.evaluate("navigator.storage.estimate().then((e) => e.usage)")
    said = file_open(page, packs["over"], "over.dmdpack")
    msg = page.inner_text("#s-err")
    check(rows <= 3 and said == "refused" and "4,096 at most" in msg,
          f"/studio/ open limits: a pack naming 4,000 absent records writes no rows for them ({rows} rows in all; storage {(est1 - est0) / 1e6:.1f} MB more); a file whose packs name 5,000 is refused ('{msg[:60]}')")
    page.context.close()


def studio_open_limits2(browser, base: str, site: Path, errors: list[str]) -> None:
    """The Studio's own split backup, with packs I was given naming
    thousands of my stamps across its parts, opens again when its parts
    are picked together; Used in names a pack that arrived before the
    record it names."""
    page = s_page(browser, base, site, errors)
    kev(page, """const s = await m.LocalStore.open(); const a = new Uint8Array(64); const ids = [];
        for (let i = 0; i < 4300; i++) { a[i % 64] = 1; const r = m.record('stamp', 'S' + i, { w: 8, h: 8, colours: [{ k: 1, hex: '#ff0000' }], frames: [{ px: await m.packPx(a) }] }); await s.put(r); ids.push({ id: r.id, rev: 1 }); }
        for (let k = 0; k < 3; k++) await s.put(m.record('pack', 'Given ' + k, { members: ids.slice(k * 1400, k * 1400 + 2000) }));
        s.db.close();""")
    # A file whose packs name 6,000 of the stamps this browser holds: none of them missing.
    held = kev(page, """const s = await m.LocalStore.open(); const ids = (await s.list({ type: 'stamp' })).items.map((h) => ({ id: h.id, rev: h.rev })); s.db.close();
        const packs = []; for (let k = 0; k < 3; k++) { const p = m.record('pack', 'Names yours ' + k, { members: ids.slice(k * 1400, k * 1400 + 2000) }); p.hash = await m.hashOf(p.body); packs.push(p); }
        const root = await K.stamp('Carrier'); return K.file(root, [root, ...packs]);""")
    held_said = file_open(page, held, "names-yours.dmdpack")
    if held_said == "asked":
        page.click("#s-fs[open] button:has-text('Cancel')")
    files = backup_files(page, 10000)
    page.context.close()
    page = s_page(browser, base, site, errors)
    tmp = Path(tempfile.mkdtemp(prefix="site-smoke-parts-"))
    paths = []
    for n_, d in files:
        (tmp / n_).write_bytes(d)
        paths.append(str(tmp / n_))
    page.evaluate("delete document.getElementById('studio').dataset.opened")
    with page.expect_file_chooser(timeout=20000) as fc:
        page.click("#s-openf")
    fc.value.set_files(paths)
    wait_js(page, "['asked', 'refused'].includes(document.getElementById('studio').dataset.opened)", timeout=120000)
    said = s_data(page)["opened"]
    if said == "asked":
        page.click("#s-fs[open] button:has-text('Add')")
        wait_js(page, "(document.getElementById('studio').dataset.opened || '').startsWith('added:')", timeout=180000)
    live = page.evaluate(S_LIVE)
    shutil.rmtree(tmp, ignore_errors=True)
    check(held_said == "asked" and len(files) == 2 and said == "asked" and live["stamp"] == 4300 and live["pack"] == 3,
          f"/studio/ open limits: a file whose packs name 6,000 stamps held here opens ({held_said}); a split backup whose given packs name 6,000 stamps across its parts opens again, picked together ({len(files)} files, {said}, {live})")
    page.context.close()
    # Used in: a pack that came before the record it names.
    page = s_page(browser, base, site, errors)
    f = kev(page, """const st = await K.stamp('Late stamp'); const pk = await K.pack('Came first', [st]); const carrier = await K.stamp('Carrier');
        // (the pack carried as a record, its member absent; the file's root a stamp of its own)
        return { pk_only: K.file(carrier, [carrier, pk]), st_only: K.file(st, [st]) };""")
    pk_only, st_only = f["pk_only"], f["st_only"]
    said1 = file_open(page, pk_only, "pack.dmdpack")
    msg1 = page.inner_text("#s-err") if said1 == "refused" else ""
    if said1 == "asked":
        file_add(page)
    file_open(page, st_only, "stamp.dmdart")
    file_add(page)
    lib_open(page)
    lib_kind(page, "stamp")
    lib_detail(page, "Late stamp")
    used = page.evaluate("[...document.querySelectorAll('#s-ldet dt')].find((d) => d.textContent === 'Used in')?.nextElementSibling?.textContent || ''")
    check("Came first" in used, f"/studio/ open limits: Used in names a pack that arrived before the record it names ({said1} {msg1[:40]}; '{used[:60]}')")
    page.context.close()


S_REFS = """async () => { const m = await import('/studio/store.js'); const s = await m.LocalStore.open();
    const n = await new Promise((ok) => { const q = s.db.transaction('refs').objectStore('refs').count(); q.onsuccess = () => ok(q.result); }); s.db.close(); return n; }"""


def studio_held_members(browser, base: str, site: Path, errors: list[str]) -> None:
    """A stranger's file whose packs name thousands of this browser's own
    stamps: the members summed over every pack of one opening are capped at
    65,536 (each would be a row in the store), the lookups go in batches
    with "Checking… n of m" on screen, the page stays live, and one under
    the cap opens and adds its rows in seconds, not minutes."""
    page = s_page(browser, base, site, errors)
    kev(page, """const s = await m.LocalStore.open(); const a = new Uint8Array(64); const ids = [];
        for (let i = 0; i < 4000; i++) { a[i % 64] = 1; const r = m.record('stamp', 'S' + i, { w: 8, h: 8, colours: [{ k: 1, hex: '#ff0000' }], frames: [{ px: await m.packPx(a) }] }); await s.put(r); ids.push({ id: r.id, rev: 1 }); }
        s.db.close(); window.__mine = ids;""")
    page.evaluate("""() => { window.__said = []; new MutationObserver(() => __said.push(document.getElementById('s-saved').textContent))
        .observe(document.getElementById('s-saved'), { childList: true, characterData: true, subtree: true }); }""")

    def stranger(packs: int, away: int = 0) -> str:
        return kev(page, """const ids = [...window.__mine]; for (let i = 0; i < A.away; i++) ids.push({ id: m.newId('stamp'), rev: 1 });
            const packs = []; for (let k = 0; k < A.packs; k++) { const p = m.record('pack', 'Names yours ' + k, { members: ids.slice(k * 7 % 64, k * 7 % 64 + 4000) }); p.hash = await m.hashOf(p.body); packs.push(p); }
            const root = await K.stamp('Carrier'); return K.file(root, [root, ...packs]);""", {"packs": packs, "away": away})
    big = stranger(17)  # 68,000 members in all
    page.evaluate("window.__beats = 0; window.__iv = setInterval(() => __beats++, 100)")
    t = time.time()
    said = file_open(page, big, "names-yours.dmdpack")
    dt = time.time() - t
    msg = page.inner_text("#s-err") if said == "refused" else ""
    check(said == "refused" and "65,536" in msg and dt < 10, f"/studio/ held members: a file whose packs name 68,000 records in all is refused at the cap ({said}, {dt:.1f} s, '{msg[:90]}')")
    if said == "asked":
        page.click("#s-fs[open] button:has-text('Cancel')")
    page.evaluate("window.__said = []; window.__beats = 0")
    ok = stranger(15, away=200)  # 60,000 members: 4,000 held here, 200 nowhere (each pack names 4,000 of the 4,200)
    t = time.time()
    said = file_open(page, ok, "names-yours.dmdpack")
    dt = time.time() - t
    seen = page.evaluate("window.__said")
    checking = [x for x in seen if x.startswith("Checking…")]
    if said == "asked":
        refs0 = page.evaluate(S_REFS)
        t2 = time.time()
        added = file_add(page)
        dt2 = time.time() - t2
        refs = page.evaluate(S_REFS) - refs0
    else:
        added, dt2, refs = 0, 0, 0
    beats = page.evaluate("window.__beats")
    page.evaluate("clearInterval(__iv)")
    check(said == "asked" and dt < 10 and checking and all(re.fullmatch(r"Checking… [\d,]+ of [\d,]+", x) for x in checking),
          f"/studio/ held members: 15 packs naming 60,000 records open in {dt:.1f} s, 'Checking…' shown with progress ({said}, {checking[:2]})")
    check(added == 16 and refs <= 60000 and dt2 < 60 and beats >= (dt + dt2) * 10 * 0.3,
          f"/studio/ held members: added in {dt2:.1f} s, {refs:,} reference rows (only for what is here), the page live ({added} added, {beats} beats)")
    page.context.close()


# --- Import (S2 branch 4): Create Art's converter in a sheet over the Studio ---
STUDIO_CSP = ("default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self' 'wasm-unsafe-eval'; "
              "object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'")  # the Studio's, as it was before Import
CONVERTER = ("/demo/pkg/", "/demo/pkg-art/", "/demo/js/find.js", "/demo/js/studio.js", "/create/")
I_PIECE = """async () => { const m = await import('/studio/store.js'); const s = await m.LocalStore.open(); const id = document.getElementById('studio').dataset.id;
    const r = await s.get(id); s.db.close(); if (!r) return null; const cm = new Map(r.body.colours.map((c) => [c.k, parseInt(c.hex.slice(1), 16)]));
    const layers = []; for (const l of r.body.layers) { const cels = []; for (const z of l.cels) cels.push(Array.from(await m.unpackPx(z, 4096, r.body.keys === 2), (k) => (k ? cm.get(k) ?? -1 : 0))); layers.push({ name: l.name, moves: l.moves, cels }); }
    return { id: r.id, name: r.name, hash: r.hash, ms: r.body.frames.map((f) => f.ms), plays: r.body.plays || 0, layers, from: r.from, colours: r.body.colours.length }; }"""


def i_frames(p: dict) -> list[list[int]]:
    """A piece's frames as shown (I_PIECE): each pixel the topmost layer's colour that is not off."""
    out = []
    for f in range(len(p["ms"])):
        px = [0] * 4096
        for l in p["layers"]:
            c = l["cels"][f if l["moves"] else 0]
            for i, v in enumerate(c):
                if v:
                    px[i] = v
        out.append(px)
    return out


def gif_rgb(data: bytes) -> tuple[list[list[int]], list[int]]:
    """A GIF's frames as 0xRRGGBB a pixel (black as 0, off: as the Studio takes them in) and their times."""
    im = Image.open(io.BytesIO(data))
    frames, ms = [], []
    for i in range(getattr(im, "n_frames", 1)):
        im.seek(i)
        rgba = im.convert("RGBA")
        raw = rgba.tobytes()
        frames.append([(raw[i] << 16 | raw[i + 1] << 8 | raw[i + 2]) if raw[i + 3] >= 128 else 0 for i in range(0, len(raw), 4)])
        d = im.info.get("duration", 0)
        ms.append(100 if d <= 10 else d)  # (0 or 1 hundredths: 100 ms, as browsers play them)
    return frames, ms


def i_page(browser, base: str, site: Path, errors: list[str], size: str = "laptop", ctx=None, init: str = "", handoff: bool = False) -> Page:
    """/studio/ (s_page) with /create/'s documents under their own CSP too (the frame's), the converter's sources stubbed."""
    page = s_page(browser, base, site, errors, size=size, ctx=ctx, init=init)
    i_route(page, site, base, handoff)
    return page


def i_route(page: Page, site: Path, base: str, handoff: bool = False) -> None:
    def create(r):
        resp = r.fetch()
        body = resp.text()
        if handoff:
            body = body.replace('data-handoff="off"', 'data-handoff="on"')
        r.fulfill(status=resp.status, body=body, headers={**resp.headers, "content-security-policy": csp_for(site, "/create/*")})
    page.route(lambda u: urllib.parse.urlsplit(u).path.startswith("/create/") and urllib.parse.urlsplit(u).path.endswith(("/", ".html")), create)
    png, gif = c_fixtures()
    page.c_asked = []
    c_stub(page, base, png, gif, page.c_asked)


def i_open(page: Page, how: str = "button") -> object:
    """Import opened (the button, or as it is), its frame ready: the frame."""
    if how == "button":
        page.click("#s-imp-b")
    wait_js(page, "document.getElementById('studio').dataset.import === 'ready'", timeout=60000)
    fr = page.query_selector("#s-imp-f").content_frame()
    fr.wait_for_selector("#create:not([aria-busy]) #studio .st-grid", timeout=60000)
    return fr


def i_file(fr, name: str, data: bytes, mime: str) -> None:
    fr.click("#st-src [data-v=own]")
    fr.set_input_files("#st-file", {"name": name, "mimeType": mime, "buffer": data})


def i_use(page: Page, fr, as_layer: bool = False, wait: str = "imported") -> str:
    if as_layer:
        page.check("#s-imp input[value=layer]")
    page.evaluate("delete document.getElementById('studio').dataset.imported")
    fr.click("#c-use")
    wait_js(page, "!!document.getElementById('studio').dataset.imported", timeout=60000)
    got = s_data(page)["imported"]
    if not got.startswith("refused"):
        wait_js(page, "document.getElementById('studio').dataset.import === 'closed'", timeout=10000)
        s_kept(page)
    return got


def i_fixtures() -> dict:
    """A 128x32 still (noise, 12 colours), a 5-frame 128x32 GIF with its own times, a 16x16 PNG with a see-through pixel."""
    import random
    rnd = random.Random(7)
    pal = [(rnd.randrange(30, 256), rnd.randrange(256), rnd.randrange(256)) for _ in range(12)]
    still = Image.new("RGB", (128, 32))
    still.putdata([pal[rnd.randrange(12)] for _ in range(4096)])
    b = io.BytesIO()
    still.save(b, "PNG")
    frames = []
    for i in range(5):
        f = Image.new("RGB", (128, 32), (0, 0, 0))
        f.putdata([pal[rnd.randrange(12)] if rnd.random() < .4 else (0, 0, 0) for _ in range(4096)])
        frames.append(f)
    g = io.BytesIO()
    frames[0].save(g, "GIF", save_all=True, append_images=frames[1:], duration=[100, 200, 50, 300, 120], loop=0, optimize=False, disposal=1)
    small = Image.new("RGBA", (16, 16), (0, 0, 0, 0))
    small.putdata([(*pal[i % 12], 255) if i != 17 else (0, 0, 0, 0) for i in range(256)])
    sm = io.BytesIO()
    small.save(sm, "PNG")
    return {"still": b.getvalue(), "anim": g.getvalue(), "small": sm.getvalue(), "pal": pal}


def studio_import_fetches(browser, base: str, site: Path, errors: list[str]) -> None:
    """The Studio drawn on, saved and downloaded without opening Import
    fetches no file of the converter, nor import.js or paste.js; its CSP
    header is byte for byte what it was; /create/'s frame-ancestors is
    'self' (no other site may frame it), the rest of its policy as it was."""
    check(csp_for(site, "/studio/*") == STUDIO_CSP, f"/studio/ import: the Studio's CSP byte for byte as it was ({csp_for(site, '/studio/*')[-40:]})")
    cc = csp_for(site, "/create/*")
    check("frame-ancestors 'self'" in cc and "frame-ancestors 'none'" not in cc and "connect-src 'self' https:" in cc,
          f"/studio/ import: /create/ may be framed by this site alone ({cc[-60:]})")
    page = i_page(browser, base, site, errors)
    page.mouse.click(*s_at(page, 5, 5))
    s_kept(page)
    with page.expect_download(timeout=60000):
        page.click("#s-gif")
    page.wait_for_timeout(500)
    got = [p for p in s_paths(page) if p.startswith(CONVERTER) or p in ("/studio/import.js", "/studio/paste.js")]
    check(not got and "/studio/out.js" in s_paths(page), f"/studio/ import: drawn, saved, downloaded: nothing of the converter fetched ({got[:3]})")
    page.context.close()


def studio_import(browser, base: str, site: Path, errors: list[str], shots: Path | None) -> None:
    """Import… opens the sheet with Create Art in its frame (one action, Use
    in the Studio; no page chrome, no Send or Download); a local still
    becomes a one-frame piece whose pixels equal the plain page's Download
    GIF of the same picture; a 5-frame GIF becomes 5 frames with its times,
    pixel for pixel; the same still as a new layer in this piece: a held
    layer, one undo step, the other layers untouched; the piece's `from`
    names it (Game art with its title and platform), and Send to panel
    offers them; Game art and GifCities search inside the frame (stubbed),
    with no CSP violation in the frame or the Studio."""
    fx = i_fixtures()
    # What the plain page's Download writes for the still.
    plain = new_page(browser, "laptop", errors)
    i_route(plain, site, base)
    plain.goto(f"{base}/create/")
    plain.wait_for_selector("#create:not([aria-busy]) #studio .st-grid", timeout=30000)
    i_file(plain, "still.png", fx["still"], "image/png")
    wait_js(plain, "document.querySelector('#st-msg').textContent.startsWith('Picture 128')", timeout=20000)
    plain.fill("#st-title", "Noise")
    wait_js(plain, "!!document.querySelector('#studio').dataset.name", timeout=5000)
    with plain.expect_download(timeout=60000) as dl:
        plain.click("#st-save")
    want_still, _ = gif_rgb(Path(dl.value.path()).read_bytes())
    check(plain.is_visible("#c-send") and plain.query_selector("#c-use") is None,
          "/create/ opened directly: as before, Send to my panel and no Use in the Studio")
    plain.context.close()

    page = i_page(browser, base, site, errors)
    page.mouse.click(*s_at(page, 2, 2))
    s_kept(page)
    first = s_data(page)["id"]
    fr = i_open(page)
    if shots:
        page.screenshot(path=str(shots / "studio-import-laptop.png"))
    chrome = fr.evaluate("""() => ({ use: !!document.getElementById('c-use'), send: !!document.getElementById('c-send'), save: !document.getElementById('st-save').hidden,
        h1: getComputedStyle(document.querySelector('#create > h1')).display, head: getComputedStyle(document.querySelector('body > header')).display })""")
    check(chrome == {"use": True, "send": False, "save": False, "h1": "none", "head": "none"}, f"/studio/ import: the sheet, the converter framed: Use in the Studio alone ({chrome})")
    i_file(fr, "still.png", fx["still"], "image/png")
    wait_js(fr, "document.querySelector('#st-msg').textContent.startsWith('Picture 128')", timeout=20000)
    got = i_use(page, fr)
    p = page.evaluate(I_PIECE)
    shown = i_frames(p)
    check(got == "piece:1" and p["id"] != first and len(p["ms"]) == 1 and len(p["layers"]) == 1 and not p["layers"][0]["moves"] and shown[0] == want_still[0]
          and p["name"] == "Imported", f"/studio/ import: a still becomes a one-frame piece, its pixels the Download GIF's ({got}, {p['name']}, {sum(a == b for a, b in zip(shown[0], want_still[0]))}/4096)")
    fr_ = p["from"]
    check(len(fr_) == 1 and fr_[0].get("external") is True and fr_[0].get("note") == "Your file", f"/studio/ import: its from says where it came from ({fr_})")
    # An animated GIF: 5 frames, their times, pixel for pixel.
    fr = i_open(page)
    i_file(fr, "anim.gif", fx["anim"], "image/gif")
    wait_js(fr, "document.querySelector('#studio').dataset.animated === '1'", timeout=20000)
    got = i_use(page, fr)
    p = page.evaluate(I_PIECE)
    src, ms = gif_rgb(fx["anim"])
    shown = i_frames(p)
    same = [sum(a == b for a, b in zip(x, y)) for x, y in zip(shown, src)]
    check(got == "piece:5" and p["ms"] == ms and same == [4096] * 5 and p["layers"][0]["moves"],
          f"/studio/ import: a 5-frame GIF becomes 5 frames with its times, pixel for pixel ({got}, {p['ms']} vs {ms}, {same})")
    # As a new layer in this piece: a held layer, one undo step, the rest untouched.
    s_new(page)
    page.mouse.click(*s_at(page, 9, 9))
    s_kept(page)
    before = page.evaluate(I_PIECE)
    steps0 = s_data(page)["steps"]
    fr = i_open(page)
    i_file(fr, "still.png", fx["still"], "image/png")
    wait_js(fr, "document.querySelector('#st-msg').textContent.startsWith('Picture 128')", timeout=20000)
    got = i_use(page, fr, as_layer=True)
    after_ = page.evaluate(I_PIECE)
    steps1 = s_data(page)["steps"]
    check(got == "layer:1" and after_["id"] == before["id"] and len(after_["layers"]) == 2 and not after_["layers"][1]["moves"]
          and after_["layers"][0]["cels"] == before["layers"][0]["cels"] and after_["layers"][1]["name"] == "Imported"
          and int(steps1.split("/")[0]) == int(steps0.split("/")[0]) + 1 and page.is_visible("#s-lrows li"),
          f"/studio/ import: as a new layer: a held layer on top, one undo step, the other layer untouched ({got}, {steps0} -> {steps1})")
    page.keyboard.press("Control+z")
    s_kept(page)
    undone = page.evaluate(I_PIECE)
    check(len(undone["layers"]) == 1 and undone["layers"][0]["cels"] == before["layers"][0]["cels"], f"/studio/ import: undo takes the layer away ({len(undone['layers'])})")
    # Game art, with its title and platform; GifCities; Send to panel offers them.
    fr = i_open(page)
    fr.click("#st-src [data-v=gameart]")
    fr.select_option("#find-platform", "SNES")
    fr.fill("#st-q", "studio test")
    fr.click("#st-go")
    fr.wait_for_selector("#find-grid .find-hit", timeout=20000)
    fr.click("#find-grid .find-hit[data-kind=Named_Titles]")
    wait_js(fr, "document.querySelector('#studio').dataset.title === 'Studio Test'", timeout=20000)
    got = i_use(page, fr)
    p = page.evaluate(I_PIECE)
    e = p["from"][0] if p["from"] else {}
    check(got == "piece:1" and e.get("note", "").startswith("Game art: Studio Test") and e.get("title") == "Studio Test" and e.get("platform") == "SNES" and p["name"] == "Studio Test",
          f"/studio/ import: Game art in the frame (stubbed): its title and platform carried in from ({e})")
    page.click("#s-send")
    page.wait_for_selector("#s-p[open]", timeout=20000)
    offered = [page.input_value("#s-p-title"), page.input_value("#s-p-platform")]
    page.click("#s-p-close")
    check(offered == ["Studio Test", "SNES"], f"/studio/ import: Send to panel offers the import's title and platform ({offered})")
    fr = i_open(page)
    fr.click("#st-src [data-v=gifcities]")
    fr.fill("#st-q", "mario")
    fr.click("#st-go")
    wait_js(fr, "document.querySelectorAll('#find-grid .find-hit').length === 2", timeout=20000)
    fr.click("#find-grid .find-hit[data-id=WIDE]")
    wait_js(fr, "document.querySelector('#studio').dataset.animated === '1'", timeout=20000)
    csp = page.evaluate("window.__csp || []") + fr.evaluate("window.__csp || []")
    got = i_use(page, fr)
    p = page.evaluate(I_PIECE)
    note = (p["from"] or [{}])[0].get("note", "")
    check(got == "piece:4" and note.startswith("GifCities: ") and not csp, f"/studio/ import: GifCities in the frame, 4 frames ({got}, '{note}', CSP {csp[:2]})")
    off = [u for u in page.c_asked if u.startswith("https://opendmd.dev/")]
    check(not off, f"/studio/ import: nothing asked of opendmd.dev {off[:2]}")
    page.context.close()


def i_png(w: int, h: int, colours: list[tuple] | None = None) -> bytes:
    im = Image.new("RGB", (w, h))
    if colours:
        im.putdata([colours[i % len(colours)] for i in range(w * h)])
    b = io.BytesIO()
    im.save(b, "PNG")
    return b.getvalue()


I_EVENT = """([kind, name, type, b64]) => { const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0)); const dt = new DataTransfer();
    dt.items.add(new File([bytes], name, { type }));
    const e = kind === 'paste' ? new ClipboardEvent('paste', { clipboardData: dt, bubbles: true, cancelable: true }) : new DragEvent('drop', { dataTransfer: dt, bubbles: true, cancelable: true });
    (kind === 'paste' ? document.body : document.getElementById('s-canvas')).dispatchEvent(e); }"""


def i_event(page: Page, kind: str, name: str, mime: str, data: bytes) -> None:
    import base64
    page.evaluate(I_EVENT, [kind, name, mime, base64.b64encode(data).decode()])


def studio_import_video(browser, base: str, site: Path, errors: list[str]) -> None:
    """Video in Import: the picker takes any video (video/*); a WebM from a
    file becomes at most 200 frames; a video past 10 s offers Start at, and
    at 3 s the first frame is the video's at 3 s; a .mov is taken."""
    page = i_page(browser, base, site, errors)
    fr = i_open(page)
    fr.click("#st-src [data-v=own]")
    accept = fr.get_attribute("#st-file", "accept")
    check(accept == "image/*,video/*", f"/studio/ import video: the picker takes any video ({accept})")
    fr.set_input_files("#st-file", str(VIDEO_FIXTURES / "clip.webm"))
    wait_js(fr, "document.querySelector('#studio').dataset.video === '1'", timeout=30000)
    n = int(fr.evaluate("document.querySelector('#studio').dataset.srcFrames"))
    got = i_use(page, fr)
    p = page.evaluate(I_PIECE)
    check(got == f"piece:{n}" and 30 <= n <= 200 and len(p["ms"]) == n, f"/studio/ import video: a WebM from a file, {n} frames ({got})")
    fr = i_open(page)
    i_file(fr, "long.webm", (VIDEO_FIXTURES / "long.webm").read_bytes(), "video/webm")
    wait_js(fr, "document.querySelector('#studio').dataset.video === '1'", timeout=60000)
    shown = fr.is_visible("#c-start")
    fr.fill("#c-start-s", "3")
    fr.evaluate("document.querySelector('#studio').dataset.video = ''")
    fr.click("#c-start-go")
    wait_js(fr, "document.querySelector('#studio').dataset.video === '1'", timeout=60000)
    n = int(fr.evaluate("document.querySelector('#studio').dataset.srcFrames"))
    got = i_use(page, fr)
    p = page.evaluate(I_PIECE)
    f0 = i_frames(p)[0]
    from collections import Counter
    top = Counter(v for v in f0 if v).most_common(1)[0][0] if any(f0) else 0
    r, g, b = top >> 16, (top >> 8) & 255, top & 255
    check(shown and got.startswith("piece:") and n <= 200 and r > 170 and g > 150 and b < 110,
          f"/studio/ import video: past 10 s, Start at; at 3 s the first frame is the video's at 3 s (yellow: #{top:06x}; {n} frames, {got})")
    fr = i_open(page)
    i_file(fr, "clip.mov", (VIDEO_FIXTURES / "clip.mp4").read_bytes(), "video/quicktime")
    with contextlib.suppress(PlaywrightTimeout):
        wait_js(fr, "document.querySelector('#studio').dataset.video === '1'", timeout=30000)
    msg = fr.inner_text("#st-msg")
    check(fr.evaluate("document.querySelector('#studio').dataset.video") == "1", f"/studio/ import video: a .mov is taken ('{msg}')")
    page.context.close()


def studio_import_paste(browser, base: str, site: Path, errors: list[str]) -> None:
    """Paste and drop: a 16x16 PNG pasted floats in exactly (its colours,
    a see-through pixel off, the converter not loaded, "Pasted" in from);
    a 640x480 one is said, with Fit it in Import, which opens the sheet with
    it; a GIF dropped on the Studio opens Import with it; a .dmdart dropped
    opens as a file, not in Import."""
    fx = i_fixtures()
    page = i_page(browser, base, site, errors)
    i_event(page, "paste", "image.png", "image/png", fx["small"])
    wait_js(page, "!!document.getElementById('studio').dataset.sel", timeout=20000)
    x0, y0 = (int(v) for v in s_data(page)["sel"].split(",")[:2])
    s_blur(page)
    page.keyboard.press("Enter")
    s_kept(page)
    px = s_pixels(page)
    pal = fx["pal"]
    want = [pal[i % 12] if i != 17 else None for i in range(256)]
    ok = all((px[(y0 + i // 16) * 128 + x0 + i % 16] == want[i]) if want[i] else px[(y0 + i // 16) * 128 + x0 + i % 16] == (0, 0, 0) for i in range(256))
    p = page.evaluate(I_PIECE)
    lit = sum(1 for v in i_frames(p)[0] if v)
    paths = s_paths(page)
    check(ok and lit == 255 and p["colours"] == 12 and {"note": "Pasted", "external": True} in p["from"] and "/studio/paste.js" in paths
          and not [q for q in paths if q.startswith(CONVERTER) or q == "/studio/import.js"],
          f"/studio/ paste: a 16 × 16 PNG floats in exactly, a see-through pixel off, the converter not loaded ({lit} lit, {p['colours']} colours, {p['from']})")
    i_event(page, "paste", "big.png", "image/png", i_png(640, 480, [(200, 40, 40), (40, 200, 40)]))
    wait_js(page, "/bigger than the panel/.test(document.getElementById('s-err').textContent)", timeout=20000)
    said = page.inner_text("#s-err")
    page.click("#s-err button")
    fr = i_open(page, how="as is")
    wait_js(fr, "document.querySelector('#st-msg').textContent.startsWith('Pasted: 640')", timeout=20000)
    check(said.startswith("This picture is 640 × 480, bigger than the panel.") and "Fit it in Import" in said, f"/studio/ paste: 640 × 480 said, and Fit it in Import opens the sheet with it ('{said[:70]}')")
    page.click("#s-imp-x")
    wait_js(page, "document.getElementById('studio').dataset.import === 'closed'", timeout=10000)
    png, gif = c_fixtures()
    i_event(page, "drop", "wide.gif", "image/gif", gif)
    fr = i_open(page, how="as is")
    wait_js(fr, "document.querySelector('#st-msg').textContent.startsWith('Your file: 88 × 31 · 4 frames')", timeout=20000)
    check(True, "/studio/ drop: a GIF dropped on the Studio opens Import with it")
    page.click("#s-imp-x")
    wait_js(page, "document.getElementById('studio').dataset.import === 'closed'", timeout=10000)
    art = kev(page, "const r = await K.piece('Dropped file'); return K.file(r, [r]);")
    page.evaluate("delete document.getElementById('studio').dataset.opened")
    i_event(page, "drop", "piece.dmdart", "", art.encode())
    wait_js(page, "['asked', 'refused'].includes(document.getElementById('studio').dataset.opened)", timeout=20000)
    check(s_data(page)["opened"] == "asked" and s_data(page)["import"] == "closed", f"/studio/ drop: a .dmdart dropped opens as a file, not in Import ({s_data(page).get('opened')})")
    page.context.close()


def studio_import_handoff(browser, base: str, site: Path, errors: list[str]) -> None:
    """The cutover's hand-off, its switch forced on here (index.html's
    data-handoff, "off" as built): /create/?q=zelda&platform=SNES,
    /create/?url=…, a POST to /create/share (the share target, through its
    worker) and bare /create/ each end in the Studio with Import open on
    the right thing. As built, the switch is off."""
    built = (site / "create/index.html").read_text()
    check('data-handoff="off"' in built, "/create/ hand-off: the switch is off as built")
    png, gif = c_fixtures()
    page = new_page(browser, "laptop", errors)
    page.add_init_script("""document.addEventListener('securitypolicyviolation', e =>
        (window.__csp = window.__csp || []).push(e.violatedDirective + ' ' + e.blockedURI))""")
    page.route(lambda u: urllib.parse.urlsplit(u).path.startswith("/studio/") and urllib.parse.urlsplit(u).path.endswith(("/", ".html")),
               lambda r: (lambda resp: r.fulfill(response=resp, headers={**resp.headers, "content-security-policy": csp_for(site, "/studio/*")}))(r.fetch()))
    i_route(page, site, base, handoff=True)
    page.route("https://cors.example/**", lambda r: r.fulfill(status=200, body=gif, headers={"content-type": "image/gif", "access-control-allow-origin": "*"}))

    def landed(path: str | None, frame_js: str) -> tuple[str, bool]:
        if path:
            page.goto(f"{base}{path}")
        page.wait_for_url(f"{base}/studio/**", timeout=30000)
        page.wait_for_selector(S_READY, timeout=30000)
        fr = i_open(page, how="as is")
        try:
            wait_js(fr, frame_js, timeout=20000)
            ok = True
        except PlaywrightTimeout:
            ok = False
        return urllib.parse.urlsplit(page.url).fragment, ok
    frag, ok = landed("/create/?q=zelda&platform=SNES", "document.getElementById('st-q').value === 'zelda' && document.getElementById('find-platform').value === 'SNES'")
    check(frag == "import?q=zelda&platform=SNES" and ok, f"/create/ hand-off: ?q=zelda&platform=SNES -> the Studio's Import on that search (#{frag}, {ok})")
    page.click("#s-imp-x")
    page.goto(f"{base}/create/?url=" + urllib.parse.quote("https://cors.example/wide.gif", safe=""))
    page.wait_for_url(f"{base}/studio/**", timeout=30000)
    page.wait_for_selector("#s-imp-fetch", timeout=30000)
    page.click("#s-imp-fetch")  # (an address from a link is fetched only when asked)
    frag, ok = landed(None, "document.querySelector('#st-msg').textContent.startsWith('cors.example: 88')")
    check(frag.startswith("import?url=https") and ok, f"/create/ hand-off: ?url= -> Import open on that picture (#{frag}, {ok})")
    page.click("#s-imp-x")
    frag, ok = landed("/create/", "true")
    check(frag == "import" and ok, f"/create/ hand-off: bare /create/ -> the Studio's Import (#{frag})")
    page.click("#s-imp-x")
    state = page.evaluate("""async () => { for (let i = 0; i < 100; i++) { const r = await navigator.serviceWorker.getRegistration('/create/');
        if (r?.active?.state === 'activated') return 'activated'; await new Promise((ok) => setTimeout(ok, 100)); } return 'none'; }""")
    page.evaluate("""(b64) => { const bytes = Uint8Array.from(atob(b64), c => c.charCodeAt(0));
        const f = document.createElement('form'); f.method = 'POST'; f.action = '/create/share'; f.enctype = 'multipart/form-data';
        const i = document.createElement('input'); i.type = 'file'; i.name = 'media'; const t = new DataTransfer();
        t.items.add(new File([bytes], 'shared.gif', { type: 'image/gif' })); i.files = t.files; f.append(i);
        document.body.append(f); f.submit(); }""", __import__("base64").b64encode(gif).decode())
    page.wait_for_url(f"{base}/studio/**", timeout=30000)
    page.wait_for_selector(S_READY, timeout=30000)
    fr = i_open(page, how="as is")
    try:
        wait_js(fr, "document.querySelector('#st-msg').textContent.startsWith('Shared: 88 × 31')", timeout=20000)
        ok = True
    except PlaywrightTimeout:
        ok = False
    check(state == "activated" and urllib.parse.urlsplit(page.url).fragment == "import?shared=1" and ok,
          f"/create/ hand-off: a file shared to the installed app (POST /create/share) -> Import open on it ({state}, {page.url}, {ok})")
    check(not page.evaluate("window.__csp || []"), f"/create/ hand-off: no CSP violation {page.evaluate('window.__csp || []')[:2]}")
    page.context.close()


def studio_import_phone(browser, base: str, site: Path, errors: list[str], shots: Path | None) -> None:
    """On a phone the sheet fills the screen; Back closes it and the piece underneath is as it was."""
    page = i_page(browser, base, site, errors, size="phone")
    page.locator("#s-canvas").tap()
    s_kept(page)
    was = page.evaluate(I_PIECE)
    px = s_pixels(page)
    i_open(page)
    box = page.locator("#s-imp").bounding_box()
    if shots:
        page.screenshot(path=str(shots / "studio-import-phone.png"))
    page.go_back()
    wait_js(page, "document.getElementById('studio').dataset.import === 'closed'", timeout=10000)
    now = page.evaluate(I_PIECE)
    check(box["width"] >= 389 and box["height"] >= 800 and now["hash"] == was["hash"] and s_pixels(page) == px and not page.is_visible("#s-imp"),
          f"/studio/ import (phone): the sheet fills the screen ({box['width']:.0f} × {box['height']:.0f}), Back closes it, the piece as it was")
    page.context.close()


def studio_import_budget(site: Path, web_size: Path = ROOT / "tools/web-size.py") -> None:
    out = subprocess.run(["uv", "run", str(web_size), str(ROOT / "opendmd-web/www-dist"), "--share", str(ROOT / "opendmd-web/share")], capture_output=True, text=True).stdout
    line = next((l for l in out.splitlines() if l.startswith("import open")), "")
    m = re.search(r"gz\s+([\d,]+) / (\d+) KB", line)
    check(bool(m) and int(m.group(1).replace(",", "")) <= 330 * 1024 and int(m.group(2)) == 330, f"/studio/ import: web-size's import open under 330 KB ({line.strip()[:80]})")


# A frames message as the frame sends it, made in the page: `o` overrides its fields; `colours` random colours over its frames.
I_MSG = """(o) => { const k = new URLSearchParams(document.getElementById('s-imp-f')?.src.split('?')[1] || location.search).get('k');
    const n = o.n ?? 1, f = o.frames ?? n, px = new Uint8Array(f * 4096 * 4);
    for (let i = 0; i < f * 4096; i++) { const c = o.colours ? (i * 2654435761) % o.colours + 1 : 0xf7a531; px.set([c >> 16 & 255, c >> 8 & 255, c & 255, 255], i * 4); }
    const m = { type: 'opendmd-studio-import', k, w: 128, h: 32, n, total: n, ms: Array(n).fill(100), plays: 0, px: px.buffer, title: '', platform: '', author: '', logo: false, note: 'Forged', ...o.set };
    delete m.frames; return m; }"""


def i_post(target, o: dict, where: str = "parent", origin: str = "location.origin") -> None:
    """A frames message posted from `target` (a page or frame) to `where` (an expression for the Studio's window)."""
    target.evaluate(f"(o) => {{ const m = ({I_MSG})(o); {where}.postMessage(m, {origin}, [m.px]); }}", o)


def studio_import_red(browser, base: str, site: Path, errors: list[str]) -> None:
    """Import's hostile inputs: a frames message from another window, from
    another origin, or from the frame before it said ready: ignored (and
    the same message from the frame, ready, taken: the control); messages
    that lie (frame count against the data, 300 frames, a time of 0, a frame
    of the wrong size, 70,000 colours, times of the wrong count, a play
    count that isn't one, a title past its cap): each refused with a
    sentence, nothing made; /create/?in=studio opened directly is the plain
    page, and another origin can't frame /create/; the sheet closed while a
    video is read: nothing lands; Import opened twice at once: one frame,
    one piece; `#import?url=` with javascript:, data: and http: addresses:
    only https: goes on."""
    page = i_page(browser, base, site, errors)
    page.mouse.click(*s_at(page, 3, 3))
    s_kept(page)
    drafts = lambda: len(page.evaluate(S_STORE, ["list", None]))
    n0 = drafts()
    fr = i_open(page)
    k = fr.evaluate("new URLSearchParams(location.search).get('k')")
    # Another window of this origin (a page of the site in a second frame), with the right nonce.
    page.evaluate("""(u) => new Promise((ok) => { const f = document.createElement('iframe'); f.id = 'x-other'; f.src = u; f.onload = ok; document.body.append(f); })""", f"{base}/licenses/")
    other = page.query_selector("#x-other").content_frame()
    i_post(other, {"set": {"k": k}})
    # Another origin (localhost is not 127.0.0.1): a window the Studio opened, posting to its opener, to any origin.
    port = urllib.parse.urlsplit(base).port
    page.context.route(f"http://localhost:{port}/x-forge", lambda r: r.fulfill(body="<!doctype html><title>x</title>", content_type="text/html"))
    with page.expect_popup() as pop:
        page.evaluate("(u) => { window.__far = open(u); }", f"http://localhost:{port}/x-forge")
    far = pop.value
    far.wait_for_load_state()
    i_post(far, {"set": {"k": k}}, where="opener", origin="'*'")
    page.wait_for_timeout(800)
    forged = s_data(page).get("imported")
    check(not forged and drafts() == n0, f"/studio/ import red: a frames message from another window or another origin is ignored ({forged}, {drafts()} drafts)")
    # The control: the same message from the frame, ready, is taken.
    page.evaluate("document.getElementById('x-other').remove()")
    far.close()
    i_post(fr, {})
    wait_js(page, "!!document.getElementById('studio').dataset.imported", timeout=20000)
    s_kept(page)
    check(s_data(page).get("imported") == "piece:1" and drafts() == n0 + 1, f"/studio/ import red: the control, the frame's own message, is taken ({s_data(page).get('imported')})")
    # Lies, from the frame after its ready.
    lies = [("a frame count against the data", {"n": 2, "frames": 1}), ("300 frames", {"n": 300}), ("a time of 0", {"set": {"ms": [0]}}),
            ("frames of 64 × 32", {"set": {"w": 64}}), ("70,000 colours", {"n": 20, "colours": 70000}), ("times of the wrong count", {"n": 2, "set": {"ms": [100]}}),
            ("a play count of -1", {"set": {"plays": -1}}), ("a title of 500 characters", {"set": {"title": "x" * 500}}), ("fewer frames in all than it holds", {"n": 2, "set": {"total": 1}}),
            ("pictures not in an ArrayBuffer", {"set": {"px": None}}), ("a time of 1.5e9", {"set": {"ms": [1.5e9]}})]
    n1 = drafts()
    was = page.evaluate(I_PIECE)
    for what, o in lies:
        fr = i_open(page)
        page.evaluate("delete document.getElementById('studio').dataset.imported")
        if o.get("set", {}).get("px", 1) is None:
            fr.evaluate(f"(o) => {{ const m = ({I_MSG})(o); m.px = [1, 2, 3]; parent.postMessage(m, location.origin); }}", {})
        else:
            i_post(fr, o)
        wait_js(page, "!!document.getElementById('studio').dataset.imported", timeout=20000)
        said = page.inner_text("#s-err")
        check(s_data(page)["imported"] == "refused" and said.startswith("That can’t be used:") and said.endswith("Nothing was changed.") and drafts() == n1,
              f"/studio/ import red: {what}: refused with a sentence, nothing made ('{said[:90]}')")
        page.click("#s-imp-x")
    now = page.evaluate(I_PIECE)
    check(now["hash"] == was["hash"], "/studio/ import red: the open piece untouched by every refusal")
    # As a layer, past the piece's colours: refused, nothing half-made.
    page.close()
    page.context.close()

    # Before ready: the frame's script never runs (blocked), a message from it is ignored.
    page = i_page(browser, base, site, errors)
    page.route("**/create/create.js", lambda r: r.abort())
    page.click("#s-imp-b")
    page.wait_for_timeout(1500)
    fr = page.query_selector("#s-imp-f").content_frame()
    i_post(fr, {})
    page.wait_for_timeout(800)
    check(s_data(page).get("import") == "open" and not s_data(page).get("imported"), f"/studio/ import red: a message from the frame before it said ready is ignored ({s_data(page).get('import')}, {s_data(page).get('imported')})")
    page.unroute("**/create/create.js")
    page.click("#s-imp-x")
    wait_js(page, "document.getElementById('studio').dataset.import === 'closed'", timeout=10000)
    # #import?url= with javascript:, data: and http: addresses: only https: goes on.
    for u in ("javascript:alert(1)", "data:image/gif;base64,R0lGODlh", "http://example.com/a.gif"):
        page.evaluate("(u) => { history.pushState({ imp: 1 }, '', '#import?url=' + encodeURIComponent(u)); dispatchEvent(new HashChangeEvent('hashchange')); }", u)
        wait_js(page, "/open|ready/.test(document.getElementById('studio').dataset.import)", timeout=10000)
        src = page.get_attribute("#s-imp-f", "src") or ""
        said = page.inner_text("#s-err")
        check("url=" not in src and said.startswith("Only an https:// address"), f"/studio/ import red: #import?url={u[:12]}… is not passed on ({src[-30:]}, '{said[:40]}')")
        page.click("#s-imp-x")
        wait_js(page, "document.getElementById('studio').dataset.import === 'closed'", timeout=10000)
    # The sheet closed while a video is read: nothing lands.
    n2 = drafts()
    fr = i_open(page)
    i_file(fr, "long.webm", (VIDEO_FIXTURES / "long.webm").read_bytes(), "video/webm")
    page.wait_for_timeout(1500)
    page.click("#s-imp-x")
    page.wait_for_timeout(3000)
    check(s_data(page)["import"] == "closed" and page.query_selector("#s-imp-f") is None and drafts() == n2 and not s_data(page).get("imported"),
          "/studio/ import red: the sheet closed while a video is read: the frame let go, nothing lands")
    # Import opened twice at once: one frame, one sheet, one piece from one Use.
    page.evaluate("async () => { const m = await import('/studio/import.js'); history.pushState({ imp: 1 }, '', '#import'); m.show(); m.show(); }")
    fr = i_open(page, how="as is")
    count = page.evaluate("[document.querySelectorAll('iframe').length, document.querySelectorAll('#s-imp').length]")
    png, gif = c_fixtures()
    i_file(fr, "wide.gif", gif, "image/gif")
    wait_js(fr, "document.querySelector('#studio').dataset.animated === '1'", timeout=20000)
    got = i_use(page, fr)
    page.wait_for_timeout(800)
    check(count == [1, 1] and got == "piece:4" and drafts() == n2 + 1, f"/studio/ import red: Import opened twice at once: one frame, one piece ({count}, {got}, {drafts() - n2})")
    page.context.close()

    # /create/?in=studio opened directly: the plain page. Another origin framing /create/: refused by its frame-ancestors.
    page = new_page(browser, "laptop", errors)
    i_route(page, site, base)
    page.goto(f"{base}/create/?in=studio&k=x")
    page.wait_for_selector("#create:not([aria-busy]) #studio .st-grid", timeout=30000)
    plain = page.query_selector("#c-send") is not None and page.query_selector("#c-use") is None and page.is_visible("#create > h1")
    page.route(f"http://localhost:{port}/x-frame", lambda r: r.fulfill(body=f'<!doctype html><iframe id="f" src="{base}/create/?in=studio"></iframe>', content_type="text/html"))
    page.goto(f"http://localhost:{port}/x-frame")
    page.wait_for_timeout(3000)
    inner = page.query_selector("#f").content_frame()
    framed = bool(inner and inner.query_selector("#create"))
    check(plain and not framed, f"/create/?in=studio: opened directly, the plain page ({plain}); framed by another origin, refused ({framed})")
    # (Both refusals are on purpose: the console says so.)
    errors[:] = [e for e in errors if not ("violates the following Content Security Policy directive: \"frame-ancestors 'self'\"" in e or "/create/create.js)" in e)]
    page.context.close()

    # Another tab with the same piece open: this tab's import as a layer keeps its work as a copy.
    page = i_page(browser, base, site, errors)
    page.mouse.click(*s_at(page, 4, 4))
    s_kept(page)
    b = page.context.new_page()
    b.on("pageerror", lambda e: errors.append(f"[studio b] pageerror: {e}"))
    b.goto(f"{base}/studio/")
    b.wait_for_selector(S_READY, timeout=30000)
    b.mouse.click(*s_at(b, 20, 20))
    wait_js(b, "document.getElementById('studio').dataset.saved === 'kept' && document.getElementById('studio').dataset.id === '" + s_data(page)["id"] + "'", timeout=10000)
    b.wait_for_timeout(600)
    fr = i_open(page)
    i_file(fr, "small.png", i_fixtures()["small"], "image/png")
    wait_js(fr, "document.querySelector('#st-msg').textContent.startsWith('Picture 16')", timeout=20000)
    got = i_use(page, fr, as_layer=True)
    wait_js(page, "/copy/.test(document.getElementById('s-saved').textContent) || /\\(copy\\)/.test(document.getElementById('s-name').value)", timeout=10000)
    check(got == "layer:1" and "(copy)" in page.input_value("#s-name"), f"/studio/ import red: another tab changed the piece: this tab's import kept as a copy ({got}, '{page.input_value('#s-name')}')")
    page.context.close()

    # A GIF of 300 frames: the first 256 kept, and said.
    frames = []
    for i in range(300):
        f = Image.new("RGB", (128, 32), (0, 0, 0))
        f.paste((250, 210, 30), (i % 120, 8, i % 120 + 8, 24))
        frames.append(f)
    g = io.BytesIO()
    frames[0].save(g, "GIF", save_all=True, append_images=frames[1:], duration=40, loop=0)
    page = i_page(browser, base, site, errors)
    fr = i_open(page)
    i_file(fr, "long.gif", g.getvalue(), "image/gif")
    wait_js(fr, "document.querySelector('#studio').dataset.animated === '1'", timeout=60000)
    got = i_use(page, fr)
    said = page.inner_text("#s-err")
    check(got == "piece:256" and said.startswith("That had 300 frames: the first 256 are kept"), f"/studio/ import red: a GIF of 300 frames: 256 kept, said ({got}, '{said[:60]}')")
    page.context.close()


def studio_paste_red(browser, base: str, site: Path, errors: list[str]) -> None:
    """Paste's hostile inputs: a file that says image/png and isn't one, an
    SVG, a 100-megapixel PNG (refused from its header, never decoded), and
    a picture whose colours would take the piece past 4,096: each said,
    nothing pasted."""
    page = i_page(browser, base, site, errors, init="""; window.__decoded = 0; const cib = window.createImageBitmap; window.createImageBitmap = (...a) => { window.__decoded++; return cib(...a); };""")
    import struct
    import zlib
    ihdr = struct.pack(">IIBBBBB", 10000, 10000, 8, 2, 0, 0, 0)
    huge = b"\x89PNG\r\n\x1a\n" + struct.pack(">I", 13) + b"IHDR" + ihdr + struct.pack(">I", zlib.crc32(b"IHDR" + ihdr)) + b"\0" * 64
    cases = [("not a PNG", "x.png", "image/png", os.urandom(4000), "isn’t a picture the Studio can read"),
             ("an SVG", "x.svg", "image/svg+xml", b'<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8"/></svg>', "isn’t a picture the Studio can read"),
             ("a 100 MP PNG", "huge.png", "image/png", huge, "10,000 × 10,000: too big to bring in")]
    for what, name, mime, data, want in cases:
        page.evaluate("document.getElementById('s-err').textContent = ''")
        i_event(page, "paste", name, mime, data)
        wait_js(page, "!!document.getElementById('s-err').textContent", timeout=20000)
        said = page.inner_text("#s-err")
        check(want in said and not s_data(page).get("sel") and page.evaluate("window.__decoded") == 0, f"/studio/ paste red: {what}: said, nothing pasted, nothing decoded ('{said[:70]}')")
    # Colours past the cap: a piece of 1,000 colours, a picture of 4,096 new ones.
    kev(page, """const cs = Array.from({ length: 1000 }, (_, i) => ({ k: i + 1, hex: '#' + (0x100000 + i * 7).toString(16).padStart(6, '0') }));
        const a = new Uint16Array(4096); a[0] = 1;
        const r = m.record('piece', 'Many colours', { w: 128, h: 32, keys: 2, colours: cs, frames: [{ key: '0000000a', ms: 100 }], layers: [{ key: '0000000b', name: 'Drawing', moves: false, cels: [await m.packPx(a, true)] }] });
        await K.put(r); localStorage.setItem('odmd.studio.last', r.id);""")
    page.reload()
    page.wait_for_selector(S_READY, timeout=30000)
    pending = s_data(page).get("pending")
    many = [(i >> 8 | 0x40, i & 255, (i * 7) & 255) for i in range(4096)]  # 4,096 colours, each once
    page.evaluate("document.getElementById('s-err').textContent = ''")
    i_event(page, "paste", "many.png", "image/png", i_png(128, 32, many))
    wait_js(page, "!!document.getElementById('s-err').textContent", timeout=20000)
    said = page.inner_text("#s-err")
    check("nothing was pasted" in said and not s_data(page).get("sel") and s_data(page).get("pending") == pending,
          f"/studio/ paste red: colours past 4,096: said, nothing half-pasted ('{said[:80]}', pending {pending} -> {s_data(page).get('pending')})")
    page.context.close()


def studio_import_video_red(browser, base: str, site: Path, errors: list[str]) -> None:
    """Videos that won't do, in Import: one the browser can't play, one with
    no picture, and one cut short (it stalls): each ends in a sentence,
    never a page left reading."""
    page = i_page(browser, base, site, errors)
    long = (VIDEO_FIXTURES / "long.webm").read_bytes()
    cases = [("one the browser can't play", "bad.mp4", "video/mp4", b"\0\0\0\x18ftypisom" + os.urandom(6000), "This browser can’t play that video"),
             ("one with no picture", "audio.webm", "video/webm", (VIDEO_FIXTURES / "audio-only.webm").read_bytes(), "That video has no picture"),
             ("one cut short", "cut.webm", "video/webm", long[: len(long) * 2 // 5], "")]
    for what, name, mime, data, want in cases:
        fr = i_open(page)
        i_file(fr, name, data, mime)
        t = time.time()
        try:
            wait_js(fr, "(() => { const t = document.querySelector('#st-msg').textContent; return t && !/^Reading/.test(t); })()", timeout=40000)
        except PlaywrightTimeout:
            pass
        msg = fr.inner_text("#st-msg")
        check(not msg.startswith("Reading") and (msg == want if want else bool(msg)), f"/studio/ import video red: {what}: ends in a sentence in {time.time() - t:.0f} s ('{msg[:70]}')")
        page.click("#s-imp-x")
        wait_js(page, "document.getElementById('studio').dataset.import === 'closed'", timeout=10000)
    page.context.close()


def i_closed(page: Page) -> None:
    wait_js(page, "document.getElementById('studio').dataset.import === 'closed' && !document.querySelector('#s-imp[open]')", timeout=10000)


def studio_import_history(browser, base: str, site: Path, errors: list[str]) -> None:
    """Import opened and closed three times (Close, Escape, Back; on a phone
    Back each time): each close leaves the address with no #import and one
    Back is enough; a reload after does not open Import again; the frame is
    a fresh one each time."""
    for size in ("laptop", "phone"):
        page = i_page(browser, base, site, errors, size=size)
        page.mouse.click(*s_at(page, 3, 3)) if size == "laptop" else page.locator("#s-canvas").tap()
        s_kept(page)
        frames, hashes = [], []
        for how in (("close", "escape", "back") if size == "laptop" else ("back", "back", "back")):
            i_open(page)
            frames.append(page.evaluate("document.querySelectorAll('iframe').length"))
            if how == "close":
                page.click("#s-imp-x")
            elif how == "escape":
                page.keyboard.press("Escape")
            else:
                page.go_back()
            i_closed(page)
            page.wait_for_timeout(300)
            hashes.append(page.evaluate("location.hash"))
        page.reload()
        page.wait_for_selector(S_READY, timeout=30000)
        page.wait_for_timeout(800)
        again = s_data(page).get("import")
        check(frames == [1, 1, 1] and hashes == ["", "", ""] and again in (None, "closed") and not page.is_visible("#s-imp"),
              f"/studio/ import ({size}): opened and closed three times, each close leaves no #import, a reload opens nothing ({hashes}, {frames}, {again})")
        if size == "phone":
            i_open(page)
            h = page.evaluate("document.querySelector('#s-imp .s-hd').getBoundingClientRect().height")
            check(h <= 96, f"/studio/ import (phone): the sheet's header in two rows at most ({h:.0f} px)")
            page.go_back()
            i_closed(page)
        page.context.close()


def studio_import_review(browser, base: str, site: Path, errors: list[str]) -> None:
    """The review's findings: Use works with no file name to be had (a title
    and an author but no platform, a title of punctuation alone) and the
    file name's warnings are hidden; a drop on the open sheet goes into it
    (never to the browser); a reload right after an import keeps it; a
    result that lands after Close makes nothing; a frame time past a minute
    is kept at a minute; a delay of 1 hundredth is 100 ms; a JPEG whose size
    comes after 300 KB of metadata pastes; a drop in the Library goes to the
    editor; a .dmdart and a GIF dropped together: the file opens, the GIF is
    said to be left; an address from a link (#import?url=, or the frame's
    own ?url=) is fetched only when asked; gifread refuses cut-short data
    and a frame bigger than the picture; as a layer, past the piece's
    colours: refused, nothing half-made."""
    fx = i_fixtures()
    asked = []
    png, gif = c_fixtures()
    import base64

    def fresh():
        p = i_page(browser, base, site, errors)
        p.on("request", lambda r: urllib.parse.urlsplit(r.url).hostname == "cors.example" and asked.append(r.url))
        p.route("https://cors.example/**", lambda r: r.fulfill(status=200, body=gif, headers={"content-type": "image/gif", "access-control-allow-origin": "*"}))
        return p
    page = fresh()

    def part(fn):
        nonlocal page
        try:
            fn()
        except Exception as e:  # (one finding that cannot finish fails alone, and the next starts on a fresh page)
            check(False, f"/studio/ import review: {fn.__doc__} did not finish: {str(e).splitlines()[0][:120]}")
            page.context.close()
            page = fresh()

    def s0():
        """Use with no file name to be had"""
        for what, title, author in (("a title and an author, no platform", "Night sky", "someone"), ("a title of punctuation alone", "!!!", "")):
            fr = i_open(page)
            i_file(fr, "still.png", fx["still"], "image/png")
            wait_js(fr, "document.querySelector('#st-msg').textContent.startsWith('Picture 128')", timeout=20000)
            fr.fill("#st-title", title)
            fr.fill("#st-author", author)
            fr.wait_for_timeout(400)
            hidden = fr.evaluate("['st-warn', 'st-name'].every((id) => getComputedStyle(document.getElementById(id)).display === 'none')")
            got = i_use(page, fr)
            p = page.evaluate(I_PIECE)
            e = (p["from"] or [{}])[0]
            check(got == "piece:1" and hidden and e.get("title") == title and e.get("author", "") == author,
                  f"/studio/ import review: Use with {what}: taken, its fields as typed, the file name's warnings hidden ({got}, {e})")
    part(s0)

    def s1():
        """A drop on the open sheet: into it, never the browser's"""
        fr = i_open(page)
        prevented = page.evaluate("""(b64) => { const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0)); const dt = new DataTransfer(); dt.items.add(new File([bytes], 'wide.gif', { type: 'image/gif' }));
            const e = new DragEvent('drop', { dataTransfer: dt, bubbles: true, cancelable: true }); document.querySelector('#s-imp .s-hd').dispatchEvent(e); return e.defaultPrevented; }""", base64.b64encode(gif).decode())
        try:
            wait_js(fr, "document.querySelector('#st-msg').textContent.startsWith('Your file: 88')", timeout=20000)
            into = True
        except PlaywrightTimeout:
            into = False
        check(prevented and into, f"/studio/ import review: a drop on the open sheet goes into it, not to the browser ({prevented}, {into})")
        # A reload right after an import keeps it.
        got = i_use(page, fr)
        pid = s_data(page)["id"]
        page.reload()
        page.wait_for_selector(S_READY, timeout=30000)
        check(got == "piece:4" and s_data(page)["id"] == pid, f"/studio/ import review: a reload right after an import opens it ({got})")
    part(s1)

    def s2():
        """A result that lands after Close: nothing made"""
        n0 = len(page.evaluate(S_STORE, ["list", None]))
        fr = i_open(page)
        fr.evaluate(f"(o) => {{ const m = ({I_MSG})(o); parent.postMessage(m, location.origin, [m.px]); parent.setTimeout(() => parent.document.getElementById('s-imp').close(), 0); }}", {})
        page.wait_for_timeout(2000)
        check(len(page.evaluate(S_STORE, ["list", None])) == n0, "/studio/ import review: a result that lands after Close makes nothing")
        i_closed(page)
    part(s2)

    def s3():
        """Frame times: past a minute kept at a minute; 1 hundredth as 100 ms"""
        for what, dur, want in (("past a minute", 65000, 60000),):
            fs = [Image.new("RGB", (128, 32), c) for c in ((200, 40, 40), (40, 200, 40))]
            g = io.BytesIO()
            fs[0].save(g, "GIF", save_all=True, append_images=fs[1:], duration=[dur, 100], loop=0)
            fr = i_open(page)
            i_file(fr, "t.gif", g.getvalue(), "image/gif")
            wait_js(fr, "document.querySelector('#studio').dataset.animated === '1'", timeout=20000)
            got = i_use(page, fr)
            p = page.evaluate(I_PIECE) if not got.startswith("refused") else {"ms": []}
            check(got == "piece:2" and p["ms"][:1] == [want], f"/studio/ import review: a frame time {what} comes in as {want} ms ({got}, {p['ms']})")
            if got.startswith("refused"):
                page.click("#s-imp-x")
                i_closed(page)
    part(s3)

    def s4():
        """A JPEG whose size comes after 300 KB of metadata"""
        j = io.BytesIO()
        Image.new("RGB", (16, 16), (200, 120, 40)).save(j, "JPEG")
        jb = j.getvalue()
        seg = b"\xff\xef" + (65002).to_bytes(2, "big") + b"\0" * 65000
        jpeg = jb[:2] + seg * 5 + jb[2:]
        s_new(page)
        page.evaluate("document.getElementById('s-err').textContent = ''")
        i_event(page, "paste", "photo.jpg", "image/jpeg", jpeg)
        try:
            wait_js(page, "!!document.getElementById('studio').dataset.sel || !!document.getElementById('s-err').textContent", timeout=20000)
        except PlaywrightTimeout:
            pass
        check(bool(s_data(page).get("sel")), f"/studio/ import review: a JPEG with 325 KB of metadata before its size pastes ('{page.inner_text('#s-err')[:60]}')")
        page.keyboard.press("Escape")
    part(s4)

    def s5():
        """A drop in the Library: to the editor first, Import open with it"""
        lib_open(page)
        i_event(page, "drop", "wide.gif", "image/gif", gif)
        i_open(page, how="as is")
        check(s_data(page).get("view") == "editor" and page.is_hidden("#s-lib") if page.query_selector("#s-lib") else s_data(page).get("view") == "editor",
              f"/studio/ import review: a drop in the Library goes to the editor, Import open with it ({s_data(page).get('view')})")
        page.click("#s-imp-x")
        i_closed(page)
    part(s5)

    def s6():
        """A .dmdart and a GIF dropped together"""
        art = kev(page, "const r = await K.piece('Both at once'); return K.file(r, [r]);")
        page.evaluate("delete document.getElementById('studio').dataset.opened")
        page.evaluate("""([a, b64]) => { const dt = new DataTransfer(); dt.items.add(new File([a], 'piece.dmdart'));
            dt.items.add(new File([Uint8Array.from(atob(b64), (c) => c.charCodeAt(0))], 'wide.gif', { type: 'image/gif' }));
            document.getElementById('s-canvas').dispatchEvent(new DragEvent('drop', { dataTransfer: dt, bubbles: true, cancelable: true })); }""", [art, base64.b64encode(gif).decode()])
        wait_js(page, "['asked', 'refused'].includes(document.getElementById('studio').dataset.opened)", timeout=20000)
        said = page.inner_text("#s-err")
        check(s_data(page)["opened"] == "asked" and "was left" in said, f"/studio/ import review: a .dmdart and a GIF dropped together: the file opens, the GIF said to be left ('{said[:70]}')")
        page.click("#s-fs[open] button:has-text('Cancel')")
    part(s6)

    def s7():
        """An address from a link: fetched only when asked"""
        asked.clear()
        page.evaluate("(u) => { history.pushState({ imp: 1 }, '', '#import?url=' + encodeURIComponent(u)); dispatchEvent(new HashChangeEvent('hashchange')); }", "https://cors.example/wide.gif")
        fr = i_open(page, how="as is")
        page.wait_for_timeout(1500)
        before = list(asked)
        shown = page.inner_text("#s-imp-url")
        page.click("#s-imp-fetch")
        wait_js(fr, "document.querySelector('#st-msg').textContent.startsWith('cors.example: 88')", timeout=20000)
        check(not before and "https://cors.example/wide.gif" in shown and asked, f"/studio/ import review: #import?url= shows the address and fetches it only when asked ({before}, '{shown[:60]}')")
        page.click("#s-imp-x")
        i_closed(page)
        asked.clear()
        page.evaluate("(u) => new Promise((ok) => { const f = document.createElement('iframe'); f.id = 'x-url'; f.src = '/create/?in=studio&k=x&url=' + encodeURIComponent(u); f.onload = ok; document.body.append(f); })", "https://cors.example/wide.gif")
        xf = page.query_selector("#x-url").content_frame()
        xf.wait_for_selector("#c-fetch", timeout=30000)
        xf.wait_for_timeout(1000)
        before = list(asked)
        xf.click("#c-fetch")
        wait_js(xf, "document.querySelector('#st-msg').textContent.startsWith('cors.example: 88')", timeout=20000)
        check(not before and asked, f"/create/ framed: its own ?url= is fetched only when asked ({before})")
        page.evaluate("document.getElementById('x-url').remove()")
    part(s7)

    def s8():
        """gifread: cut-short data, a frame bigger than the picture"""
        g = io.BytesIO()
        Image.effect_noise((128, 32), 60).convert("RGB").save(g, "GIF")
        gb = g.getvalue()
        big = bytearray(gb)
        i = 13 + (3 << ((gb[10] & 7) + 1) if gb[10] & 0x80 else 0)  # past the header and its table, to the first image
        while big[i] == 0x21:
            i += 2
            while big[i]:
                i += big[i] + 1
            i += 1
        big[i + 5:i + 9] = (300).to_bytes(2, "little") + (300).to_bytes(2, "little")
        fs = [Image.new("RGB", (128, 32), c) for c in ((200, 40, 40), (40, 200, 40))]
        one = io.BytesIO()
        fs[0].save(one, "GIF", save_all=True, append_images=fs[1:], duration=[10, 100], loop=0)
        res = page.evaluate("""async ([a, b, c]) => { const { read } = await import('/create/gifread.js'); const t = (x) => { try { return read(Uint8Array.from(atob(x), (c) => c.charCodeAt(0))).ms[0]; } catch (e) { return e.message; } };
            return [t(a), t(b), t(c)]; }""", [base64.b64encode(gb[: len(gb) // 2]).decode(), base64.b64encode(bytes(big)).decode(), base64.b64encode(one.getvalue()).decode()])
        check("cut short" in str(res[0]) and "bigger than the picture" in str(res[1]) and res[2] == 100,
              f"/create/ gifread: cut-short data and a frame bigger than the picture refused; a delay of 1 hundredth read as 100 ms ({res})")
    part(s8)

    def s9():
        """As a layer, past the piece's colours"""
        kev(page, """const cs = Array.from({ length: 4000 }, (_, i) => ({ k: i + 1, hex: '#' + (0x200000 + i * 3).toString(16).padStart(6, '0') }));
            const a = new Uint16Array(4096); a[0] = 1;
            const r = m.record('piece', 'Full of colours', { w: 128, h: 32, keys: 2, colours: cs, frames: [{ key: '0000000a', ms: 100 }], layers: [{ key: '0000000b', name: 'Drawing', moves: false, cels: [await m.packPx(a, true)] }] });
            await K.put(r); localStorage.setItem('odmd.studio.last', r.id);""")
        page.reload()
        page.wait_for_selector(S_READY, timeout=30000)
        was = page.evaluate(I_PIECE)
        pend = s_data(page).get("pending")
        many = [(i >> 4 | 0x80, (i * 16) & 255, 7) for i in range(200)]
        fr = i_open(page)
        i_file(fr, "many.png", i_png(128, 32, many), "image/png")
        wait_js(fr, "document.querySelector('#st-msg').textContent.startsWith('Picture 128')", timeout=20000)
        got = i_use(page, fr, as_layer=True)
        said = page.inner_text("#s-err")
        now = page.evaluate(I_PIECE)
        check(got == "refused" and "more than 4,096" in said and now["hash"] == was["hash"] and len(now["layers"]) == 1 and s_data(page).get("pending") == pend,
              f"/studio/ import review: as a layer, past the piece's colours: refused, nothing half-made ('{said[:80]}', pending {pend} -> {s_data(page).get('pending')})")


    part(s9)
    page.context.close()


# --- Share links (S2 branch 5): #p= a flattened piece, #k= a small pack ---
def link_p(frames: list[list[int]], ms: list[int], colours: list[int], name: str = "", author: str = "", licence: int = 0,
           src: tuple | None = None, plays: int = 0, version: int = 1, raw_hook=None) -> str:
    """A #p= code written here, from docs/studio-format.md's layout (not the page's encoder): frames as
    indices into `colours` (1-based, 0 off)."""
    import base64
    import struct
    import zlib
    c = len(colours)
    wide = c > 255
    b = bytearray(struct.pack(">BHHH", version, len(frames), plays, c))
    for v in colours:
        b += bytes([v >> 16 & 255, v >> 8 & 255, v & 255])
    for t in ms:
        b += struct.pack(">H", t // 10)
    px = bytearray()
    for f in frames:
        for v in f:
            px += struct.pack(">H", v) if wide else bytes([v])
    w = 4096 * (2 if wide else 1)
    for i in range(len(px) - 1, w - 1, -1):
        px[i] ^= px[i - w]
    b += px
    for t in (name, author):
        e = t.encode()[:255]
        b += bytes([len(e)]) + e
    b += bytes([licence])
    if src:
        e = src[0].encode()
        b += bytes([1, len(e)]) + e + struct.pack(">I", src[1]) + bytes.fromhex(src[2].removeprefix("sha256-"))
    else:
        b += b"\0"
    if raw_hook:
        b = raw_hook(b)
    return base64.urlsafe_b64encode(zlib.compress(bytes(b), 9)).decode().rstrip("=")


def link_k(data: dict | str) -> str:
    import base64
    import zlib
    text = data if isinstance(data, str) else json.dumps(data)
    return base64.urlsafe_b64encode(zlib.compress(text.encode(), 9)).decode().rstrip("=")


L_SHOWN = "document.getElementById('studio').dataset.linkOpened"


def l_goto(page: Page, base: str, frag: str) -> str:
    """The Studio opened anew at `frag`; what link.js made of it: shown, pack, refused (or nothing)."""
    page.goto("about:blank")  # (else a new fragment is the same page, its state kept)
    page.goto(f"{base}/studio/{frag}")
    page.wait_for_selector(S_READY, timeout=30000)
    try:
        wait_js(page, f"['shown', 'pack', 'refused'].includes({L_SHOWN})", timeout=20000)
    except PlaywrightTimeout:
        return ""
    if s_data(page).get("linkOpened") == "pack":
        wait_js(page, "['asked', 'refused'].includes(document.getElementById('studio').dataset.opened)", timeout=30000)
    return s_data(page)["linkOpened"]


def l_pieces(page: Page) -> int:
    return len(page.evaluate(S_STORE, ["list", None]))


def studio_links(browser, base: str, site: Path, errors: list[str], shots: Path | None) -> None:
    """Sharing is shelved (the owner, 2026-11-12; studio.js SHARE): no Share
    button, no pack's Share link in the Library, link.js not with the page.
    A link that comes in still opens: in a fresh profile an 8-frame loop's
    #p= (written here, from docs/studio-format.md) shows the picture playing
    and who it says made it, stores nothing and fetches nothing but link.js
    until Open as a new piece, whose pixels and times are the link's; no
    request carries the fragment; from names the record the link names (not
    held here); a pack of three stamps as #k= opens as a file does, the
    link's claims shown, stored only on Add."""
    page = s_page(browser, base, site, errors)
    got = s_paths(page)
    kev(page, """const st = []; for (const n of ['Star', 'Heart']) { const r = await K.stamp(n, 4, 4); await K.put(r); st.push(r); }
        await K.put(await K.pack('Two', st));""")
    page.reload()
    page.wait_for_selector(S_READY, timeout=30000)
    shares = page.evaluate("[...document.querySelectorAll('#studio button')].filter((b) => /share/i.test(b.textContent + b.title) && b.getClientRects().length).length")
    lib_open(page)
    lib_kind(page, "pack")
    lib_detail(page, "Two")
    in_lib = page.evaluate("[...document.querySelectorAll('#s-ldet button')].filter((b) => /share|link/i.test(b.textContent) || b.dataset.la === 'link').length")
    check(page.locator("#s-share").count() == 0 and shares == 0 and in_lib == 0 and "/studio/link.js" not in got,
          f"/studio/ links: no Share button and no pack's Share link (sharing shelved; {shares}, {in_lib}; link.js not with the page)")
    page.context.close()
    # A loop's link, opened in a fresh profile.
    cols = [0xf7a531, 0x33d1ff, 0xff4f8b]
    frames = [[(1 + (i % 128 - f * 12 + i // 128) % 3) if 8 <= i // 128 < 24 and 0 <= i % 128 - f * 12 < 16 else 0 for i in range(4096)] for f in range(8)]
    src = ("pc_" + "5a" * 16, 3, "sha256-" + "c3" * 32)
    code = link_p(frames, [80] * 8, cols, name="Loop", author="tester", licence=1, src=src)
    page = s_page(browser, base, site, errors)
    asked = []
    page.on("request", lambda r: asked.append(r.url))
    n0 = l_pieces(page)
    said = l_goto(page, base, "#p=" + code)
    page.wait_for_timeout(400)
    a = page.evaluate("document.querySelector('#s-lk canvas').toDataURL()")
    page.wait_for_timeout(250)
    b = page.evaluate("document.querySelector('#s-lk canvas').toDataURL()")
    who = page.inner_text("#s-lk-who")
    others = {urllib.parse.urlsplit(u).path for u in asked} - S_FIRST - {"/studio/link.js", "/studio/"}
    check(said == "shown" and a != b and "8 frames · by “tester” · licence CC0, the link says" in who and l_pieces(page) == n0 and page.evaluate("location.hash") == "",
          f"/studio/ links: opened in a fresh profile, the picture plays, who made it as the link says, nothing stored ({said}, '{who[:60]}', {l_pieces(page) - n0})")
    check(not others and not any(code[:40] in u for u in asked) and all(urllib.parse.urlsplit(u).hostname == "127.0.0.1" for u in asked),
          f"/studio/ links: opening a link fetches nothing but link.js, and no request carries the fragment ({sorted(others)[:3]})")
    if shots:
        page.screenshot(path=str(shots / "studio-link-laptop.png"))
    page.click("#s-lk-open")
    wait_js(page, f"{L_SHOWN} === 'opened'", timeout=20000)
    s_kept(page)
    p = page.evaluate(I_PIECE)
    want = [[cols[v - 1] if v else 0 for v in f] for f in frames]
    e = (p["from"] or [{}])[0]
    check(i_frames(p) == want and p["ms"] == [80] * 8 and l_pieces(page) == n0 + 1 and e.get("id") == src[0] and e.get("rev") == src[1] and e.get("hash") == src[2],
          f"/studio/ links: Open as a new piece: its pixels and times the link's, from naming its id, rev and hash ({len(p['ms'])} frames, {e.get('id', '')[:12]}, rev {e.get('rev')})")
    page.context.close()
    # A pack of three stamps as #k= (made in one profile, never stored there), opened in another.
    page = s_page(browser, base, site, errors)
    pack = kev(page, """const st = []; for (const n of ['Star', 'Heart', 'Coin']) st.push(await K.stamp(n, 4, 4, { env: { author: 'packer', licence: 'CC-BY-4.0' } }));
        const p = await K.pack('Three', st); return K.file(p, [p, ...st]);""")
    page.context.close()
    page = s_page(browser, base, site, errors)
    n0 = page.evaluate(S_LIVE)
    said = l_goto(page, base, "#k=" + link_k(pack))
    rows = file_rows(page)
    title = page.inner_text("#s-fs h2")
    before = page.evaluate(S_LIVE)
    added = file_add(page)
    after_ = page.evaluate(S_LIVE)
    check(said == "pack" and title == "Open a shared link" and len(rows) == 4 and all("the link says" in r["text"] for r in rows)
          and before == n0 and added == 4 and after_["stamp"] == n0["stamp"] + 3 and after_["pack"] == n0["pack"] + 1,
          f"/studio/ links: a pack of three stamps as #k=, opened as a file is, the link's claims shown, stored only on Add ({added}, {after_})")
    page.context.close()


# The Studio's notice (studio.js say): every error and message in view, pinned, never at the page's foot.
N_SEEN = """() => { const n = document.getElementById('s-note'), e = document.getElementById('s-err'), x = document.getElementById('s-note-x');
    const said = document.getElementById('s-said')?.textContent, alarm = document.getElementById('s-alarm')?.textContent;
    if (!n || n.hidden || e.hidden || !(said || alarm)) return null; const r = n.getBoundingClientRect(), xr = x.getBoundingClientRect();
    const hit = document.elementFromPoint(r.left + Math.min(r.width / 2, 40), r.top + Math.min(r.height / 2, 14)), hx = document.elementFromPoint(xr.left + xr.width / 2, xr.top + xr.height / 2);
    return { inView: r.top >= 0 && r.left >= 0 && r.bottom <= innerHeight && r.right <= innerWidth && r.width > 0, onTop: !!hit && n.contains(hit) && hx === x,
             role: alarm ? 'alert' : 'status', heard: alarm || said, kind: n.dataset.kind, at: n.dataset.at, text: e.textContent, inSheet: n.closest('dialog')?.id || '',
             liveHidden: [...document.querySelectorAll('#s-said, #s-alarm')].some((l) => l.hidden || l.closest('[hidden]')), buttonsLive: !!document.querySelector('[aria-live] button') }; }"""


def n_seen(page: Page, timeout: float = 30000) -> dict:
    wait_js(page, f"!!({N_SEEN})()", timeout=timeout)
    return page.evaluate(N_SEEN)


def n_active(page: Page) -> str:
    return page.evaluate("document.activeElement?.id || document.activeElement?.tagName || ''")


def studio_notices(browser, base: str, site: Path, errors: list[str], shots: Path | None) -> None:
    """What the Studio says is where the user is looking, at laptop and phone
    sizes, light and dark: an import refusal (in the Import sheet), a file
    that won't open (the page scrolled to its foot first), and Send to panel's
    blocked pop-up (under Send), each in the window and on top (Dismiss too),
    heard by a live region that is never hidden (alert or status, no buttons
    in it); Escape dismisses it first, with focus on its control or elsewhere
    in a sheet (the sheet stays open, the tool unchanged); Dismiss is one Tab
    from Send; after Dismiss focus goes back to the control, else the
    sheet's Close, else the editor; a sheet's notice goes with the sheet;
    news clears when it is done (a stamp picked)."""
    for size, scheme in (("laptop", "light"), ("laptop", "dark"), ("phone", "light"), ("phone", "dark")):
        tag = f"/studio/ notice ({size}, {scheme})"
        page = i_page(browser, base, site, errors, size=size)
        page.emulate_media(color_scheme=scheme)
        # An import refusal, said in the open sheet.
        fr = i_open(page)
        page.evaluate("delete document.getElementById('studio').dataset.imported")
        i_post(fr, {"set": {"ms": [0]}})
        wait_js(page, "!!document.getElementById('studio').dataset.imported", timeout=20000)
        x = n_seen(page)
        check(x["inView"] and x["onTop"] and x["inSheet"] == "s-imp" and x["role"] == "alert" and x["kind"] == "bad" and x["text"].startswith("That can’t be used:")
              and x["heard"] == x["text"] and not x["liveHidden"] and not x["buttonsLive"],
              f"{tag}: an import refusal is in view, on top of the sheet, heard as an alert ({x})")
        if shots:
            page.screenshot(path=str(shots / f"studio-notice-import-{size}-{scheme}.png"))
        clear = page.evaluate("() => { const r = document.getElementById('s-imp-x').getBoundingClientRect(); return document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2)?.id === 's-imp-x'; }")
        check(clear, f"{tag}: the notice leaves the sheet's Close clear")
        # Escape in the sheet, focus elsewhere in it: the notice goes, the sheet stays.
        page.focus("#s-imp input[value=layer]")
        page.keyboard.press("Escape")
        page.wait_for_timeout(200)
        check(page.is_hidden("#s-note") and page.is_visible("#s-imp") and n_active(page) == "s-imp-x",
              f"{tag}: Escape in the sheet dismisses the notice first, the sheet stays open, focus on its Close ({n_active(page)})")
        i_post(fr, {"set": {"ms": [0]}})
        n_seen(page)
        page.click("#s-imp-x")
        wait_js(page, "document.getElementById('s-note').hidden", timeout=5000)
        check(page.is_hidden("#s-note"), f"{tag}: the sheet's notice goes with the sheet")
        # A file that won't open, the page scrolled to its foot; Escape with focus on its button.
        if page.is_visible("#s-tabs"):
            page.click("#s-tabs [data-tab=piece]")
        page.evaluate("scrollTo(0, document.body.scrollHeight)")
        said = file_open(page, b"not a studio file", "junk.dmdart")
        page.evaluate("scrollTo(0, document.body.scrollHeight)")
        page.wait_for_timeout(100)
        x = n_seen(page)
        check(said == "refused" and x["inView"] and x["onTop"] and not x["inSheet"] and x["role"] == "alert" and "isn’t a Studio file" in x["text"],
              f"{tag}: a file that won't open is said in view, the page scrolled to its foot ({said}, {x})")
        if shots:
            page.screenshot(path=str(shots / f"studio-notice-file-{size}-{scheme}.png"))
        page.click(".s-tool[data-tool=stamp]")
        page.evaluate("document.activeElement?.blur()")
        file_open(page, b"not a studio file", "junk.dmdart")
        n_seen(page)
        page.focus("#s-openf")
        page.keyboard.press("Escape")
        tool = page.evaluate("document.querySelector('.s-tool[aria-pressed=true]')?.dataset.tool")
        check(page.is_hidden("#s-note") and tool == "stamp" and n_active(page) == "studio",
              f"{tag}: Escape with focus on Open a file… dismisses the notice first (the stamp tool kept: {tool}), focus to the editor ({n_active(page)})")
        page.click(".s-tool[data-tool=pencil]")
        # News: a status; gone once a stamp is picked.
        page.evaluate("document.activeElement?.blur()")
        page.keyboard.press("s")
        x = n_seen(page)
        check(x["role"] == "status" and x["kind"] == "info" and x["inView"] and "Pick a stamp" in x["text"], f"{tag}: news is a status, in view ({x})")
        if page.is_visible("#s-tabs"):
            page.click("#s-tabs [data-tab=stamps]")
        page.wait_for_selector("#s-stampsx [data-n]", timeout=20000)
        page.click("#s-stampsx [data-n]")
        check(page.is_hidden("#s-note"), f"{tag}: news clears once what it asked is done (a stamp picked)")
        page.keyboard.press("b")
        # Send to panel: the pop-up blocked (the address one that is never opened: window.open answers null).
        if page.is_visible("#s-tabs"):
            page.click("#s-tabs [data-tab=colours]")
        page.mouse.click(*s_at(page, 4, 4))
        s_kept(page)
        page.evaluate("localStorage.setItem('odmd.display', '127.0.0.1:9'); window.open = () => null")
        page.click("#s-send")
        page.wait_for_selector("#s-p-send:not([disabled])", timeout=120000)
        page.click("#s-p-send")
        x = n_seen(page)
        check(x["inView"] and x["onTop"] and x["inSheet"] == "s-p" and x["role"] == "alert" and "allow pop-ups" in x["text"] and x["at"] == "control",
              f"{tag}: Send to panel's blocked pop-up is said under Send, in view ({x})")
        if shots:
            page.screenshot(path=str(shots / f"studio-notice-send-{size}-{scheme}.png"))
        page.focus("#s-p-send")
        page.keyboard.press("Tab")
        tab = n_active(page)
        page.keyboard.press("Enter")
        check(tab == "s-note-x" and page.is_hidden("#s-note") and n_active(page) == "s-p-send" and page.is_visible("#s-p"),
              f"{tag}: Dismiss is the next Tab from Send; after it focus is back on Send, the sheet open ({tab}, {n_active(page)})")
        page.click("#s-p-send")
        n_seen(page)
        page.focus("#s-p-send")
        page.keyboard.press("Escape")
        page.wait_for_timeout(200)
        check(page.is_hidden("#s-note") and page.is_visible("#s-p"), f"{tag}: Escape with focus on Send dismisses the notice, the sheet stays")
        page.context.close()


def studio_links_red(browser, base: str, site: Path, errors: list[str]) -> None:
    """Hostile links: too long (refused before decoding), bad base64, a
    deflate bomb (inflating stops at the cap), counts and sizes that lie,
    an unknown version, a #p= that is really a pack, a pixel with no colour,
    names that aren't text or are markup (shown as text), __proto__ keys, a
    lying hash in a #k=, links naming the user's own ids (their records
    untouched), links with #import or #library; 500 random, truncated and
    bit-flipped fragments: each ends at a sentence, never an exception, and
    nothing is ever stored by opening one."""
    import random
    import zlib
    import base64
    page = s_page(browser, base, site, errors)
    page.mouse.click(*s_at(page, 2, 2))
    s_kept(page)
    mine = page.evaluate(S_STORE, ["get", None])["r"]
    asked = []
    page.on("request", lambda r: urllib.parse.urlsplit(r.url).hostname != "127.0.0.1" and asked.append(r.url))
    one = [[1 if i % 5 == 0 else 0 for i in range(4096)]]
    good = link_p(one, [100], [0xf7a531], name="Ok")
    n0 = l_pieces(page)
    cases = [
        ("9,000 characters", "#p=" + "A" * 9000, "more than a Studio link holds"),
        ("100,000 characters", "#k=" + "A" * 100000, "more than a Studio link holds"),
        ("bad base64", "#p=@@!!", "damaged"),
        ("base64 that doesn't unpack", "#p=AAAA", "damaged"),
        ("a deflate bomb", "#p=" + base64.urlsafe_b64encode(zlib.compress(b"\1" + b"\0" * (3 << 20), 9)).decode().rstrip("="), "damaged"),
        ("a #k= bomb", "#k=" + base64.urlsafe_b64encode(zlib.compress(b" " * (5 << 20), 9)).decode().rstrip("="), "damaged"),
        ("an unknown version", "#p=" + link_p(one, [100], [0xf7a531], version=2), "version"),
        ("300 frames", "#p=" + link_p(one, [100], [0xf7a531], raw_hook=lambda b: b[:1] + (300).to_bytes(2, "big") + b[3:]), "damaged"),
        ("5,000 colours", "#p=" + link_p(one, [100], [0xf7a531], raw_hook=lambda b: b[:5] + (5000).to_bytes(2, "big") + b[7:]), "damaged"),
        ("2 frames with one frame's data", "#p=" + link_p(one, [100], [0xf7a531], raw_hook=lambda b: b[:1] + (2).to_bytes(2, "big") + b[3:]), "damaged"),
        ("bytes past its end", "#p=" + link_p(one, [100], [0xf7a531], raw_hook=lambda b: b + b"xyz"), "more than it says"),
        ("a pixel with no colour", "#p=" + link_p([[5] * 4096], [100], [0xf7a531]), "no colour"),
        ("a time of 0", "#p=" + link_p(one, [0], [0xf7a531]), "0 ms"),
        ("a licence that isn't one", "#p=" + link_p(one, [100], [0xf7a531], licence=9), "licence"),
        ("a name that isn't text", "#p=" + link_p(one, [100], [0xf7a531], raw_hook=lambda b: b[:-4] + b"\2\xff\xfe\0\0"), "isn’t text"),
        ("a colour listed twice", "#p=" + link_p(one, [100], [0xf7a531, 0xf7a531]), "twice"),
        ("a source that isn't a piece", "#p=" + link_p(one, [100], [0xf7a531], src=("pl_" + "0" * 32, 1, "sha256-" + "0" * 64)), "source"),
        ("a #p= that is really a pack", "#p=" + link_k({"format": "opendmd-studio/1", "root": "x", "records": []}), "damaged"),
        ("#p= with #import after it", "#p=" + good + "#import", "damaged"),
    ]
    for what, frag, want in cases:
        said = l_goto(page, base, frag)
        msg = page.inner_text("#s-err")
        check(said == "refused" and want in msg and l_pieces(page) == n0 and not page.is_visible("#s-lk") and not page.is_visible("#s-imp"),
              f"/studio/ links red: {what}: refused with a sentence, nothing stored ('{msg[:70]}')")
        if "bomb" in what:
            got = page.evaluate("import('/studio/store.js').then((m) => [m.inflated.read, m.inflated.kept])")
            check(got[0] < (3 << 20) + (256 << 10) and len(frag) <= 8003, f"/studio/ links red: {what} ({len(frag):,} characters): inflating stopped at the cap ({got[0]:,} B read)")
    # Markup as names: shown as text.
    said = l_goto(page, base, "#p=" + link_p(one, [100], [0xf7a531], name='<img src=x onerror="window.__x=1">', author="<b>me</b>"))
    h2 = page.inner_text("#s-lk-name")
    check(said == "shown" and h2 == 'Called “<img src=x onerror="window.__x=1">”' and "by “<b>me</b>”" in page.inner_text("#s-lk-who") and not page.evaluate("window.__x"),
          "/studio/ links red: a name and an author of markup, shown as text")
    page.click("#s-lk button:has-text('Close')")
    # A #p= naming my own piece, with other content: my record untouched, the new piece's from not naming it.
    said = l_goto(page, base, "#p=" + link_p(one, [100], [0xf7a531], name="Mine?", src=(mine["id"], mine["rev"], "sha256-" + "ab" * 32)))
    page.click("#s-lk-open")
    wait_js(page, f"{L_SHOWN} === 'opened'", timeout=20000)
    s_kept(page)
    p = page.evaluate(I_PIECE)
    again = page.evaluate(S_STORE, ["get", mine["id"]])["r"]
    check(p["id"] != mine["id"] and again["hash"] == mine["hash"] and again["rev"] == mine["rev"] and not any(f.get("id") == mine["id"] for f in p["from"]),
          f"/studio/ links red: a #p= naming my piece with other content: mine untouched, it isn't named ({p['from']})")
    n1 = l_pieces(page)
    # #k=: a lying hash, __proto__, my own id with other content.
    recs = kev(page, """const st = await K.stamp('Liar'); const bad = { ...st, hash: 'sha256-' + '0'.repeat(64) };
        const proto = JSON.parse(JSON.stringify(await K.stamp('Proto'))); const raw = JSON.stringify({ format: m.FORMAT, root: proto.id, records: [proto] }).replace('"name":"Proto"', '"name":"Proto","__proto__":{"polluted":1},"constructor":{"prototype":{"polluted":1}}');
        const mineCopy = await K.piece('Not yours', [[9, 9, 2]], { set: { id: A } });
        return { lie: K.file(bad, [bad]), proto: raw, mine: K.file(mineCopy, [mineCopy]) };""", mine["id"])
    said = l_goto(page, base, "#k=" + link_k(recs["lie"]))
    msg = page.inner_text("#s-err")
    check(said == "pack" and s_data(page).get("opened") == "refused" and "isn’t what its hash says" in msg, f"/studio/ links red: a #k= with a lying hash: refused ('{msg[:60]}')")
    said = l_goto(page, base, "#k=" + link_k(recs["proto"]))
    if s_data(page).get("opened") == "asked":
        page.click("#s-fs[open] button:has-text('Cancel')")
    check(not page.evaluate("({}).polluted || Object.prototype.polluted") and l_pieces(page) == n1, f"/studio/ links red: __proto__ and constructor keys pollute nothing ({s_data(page).get('opened')})")
    said = l_goto(page, base, "#k=" + link_k(recs["mine"]))
    how = [r["how"] for r in file_rows(page)] if s_data(page).get("opened") == "asked" else []
    if how:
        file_add(page)
    again = page.evaluate(S_STORE, ["get", mine["id"]])["r"]
    check(how == ["copy"] and again["hash"] == mine["hash"], f"/studio/ links red: a #k= with my piece's id and other content comes in as a copy, mine untouched ({how})")
    # With #import or #library.
    n2 = l_pieces(page)
    page.goto("about:blank")
    page.goto(f"{base}/studio/#import?p={good}")
    page.wait_for_selector(S_READY, timeout=30000)
    wait_js(page, "/open|ready/.test(document.getElementById('studio').dataset.import)", timeout=20000)
    imp_ok = not s_data(page).get("linkOpened") and not page.is_visible("#s-lk") and "p=" not in (page.get_attribute("#s-imp-f", "src") or "")
    page.click("#s-imp-x")
    page.goto("about:blank")
    page.goto(f"{base}/studio/#library&k={link_k(recs['mine'])}")
    page.wait_for_selector(S_READY, timeout=30000)
    page.wait_for_timeout(1500)
    lib_ok = not s_data(page).get("linkOpened") and not page.is_visible("#s-fs") and not page.is_visible("#s-lk")
    check(imp_ok and lib_ok and l_pieces(page) == n2, f"/studio/ links red: #import?p= and #library&k= open no link ({imp_ok}, {lib_ok})")
    # 500 random, truncated and bit-flipped fragments, in the page's own reader.
    rnd = random.Random(5)
    frags = []
    for i in range(500):
        k = i % 3
        if k == 0:
            frags.append(base64.urlsafe_b64encode(rnd.randbytes(rnd.randrange(1, 3000))).decode().rstrip("="))
        elif k == 1:
            frags.append(good[: rnd.randrange(1, len(good))])
        else:
            z = bytearray(base64.urlsafe_b64decode(good + "=" * (-len(good) % 4)))
            for _ in range(rnd.randrange(1, 4)):
                z[rnd.randrange(len(z))] ^= 1 << rnd.randrange(8)
            frags.append(base64.urlsafe_b64encode(bytes(z)).decode().rstrip("="))
    res = page.evaluate("""async (fs) => { const m = await import('/studio/link.js'); const out = { damaged: 0, read: 0, other: [] };
        for (const f of fs) { try { await m.decodePiece(f); out.read++; } catch (e) { if (/^This link is damaged/.test(e.message)) out.damaged++; else out.other.push(e.message); } } return out; }""", frags)
    check(res["damaged"] + res["read"] == 500 and not res["other"] and l_pieces(page) == n2,
          f"/studio/ links red: 500 random, truncated and bit-flipped fragments: each damaged or a piece, never an exception ({res['damaged']} damaged, {res['read']} read, {res['other'][:2]})")
    check(not asked, f"/studio/ links red: opening links fetched nothing from anywhere else {asked[:2]}")
    page.context.close()


def studio_links_leftovers(browser, base: str, site: Path, errors: list[str]) -> None:
    """From the import review: framed /create/ keeps every drop from the
    browser (one outside the converter loads); Use never touches the title
    and author fields as it writes the GIF."""
    page = i_page(browser, base, site, errors)
    fr = i_open(page)
    png, gif = c_fixtures()
    import base64
    prevented = fr.evaluate("""(b64) => { const dt = new DataTransfer(); dt.items.add(new File([Uint8Array.from(atob(b64), (c) => c.charCodeAt(0))], 'w.gif', { type: 'image/gif' }));
        const o = new DragEvent('dragover', { dataTransfer: dt, bubbles: true, cancelable: true }); document.body.dispatchEvent(o);
        const e = new DragEvent('drop', { dataTransfer: dt, bubbles: true, cancelable: true }); document.querySelector('#c-cap').dispatchEvent(e); return o.defaultPrevented && e.defaultPrevented; }""",
                           base64.b64encode(gif).decode())
    try:
        wait_js(fr, "document.querySelector('#st-msg').textContent.startsWith('Your file: 88')", timeout=20000)
        loaded = True
    except PlaywrightTimeout:
        loaded = False
    check(prevented and loaded, f"/create/ framed: a drop outside the converter is the page's, never the browser's, and loads ({prevented}, {loaded})")
    fr.fill("#st-title", "Night sky")
    fr.fill("#st-author", "someone")
    fr.evaluate("""() => { parent.__seen = new Set(); const t = document.getElementById('st-title'), a = document.getElementById('st-author');
        const look = () => parent.__seen.add(t.value + '|' + a.value); setInterval(look, 2); t.addEventListener('input', look); a.addEventListener('input', look); }""")
    got = i_use(page, fr)
    seen = page.evaluate("[...window.__seen]")
    check(got == "piece:4" and seen == ["Night sky|someone"], f"/create/ framed: Use writes the GIF without touching the title and author ({got}, {seen})")
    page.context.close()


def studio_links_review(browser, base: str, site: Path, errors: list[str]) -> None:
    """The links review: a second link stops the first one's
    playing, and after Close nothing paints; bidi and zero-width controls are
    dropped from a link's and a file's names and authors, shown and stored,
    the claims quoted and isolated; a link naming a record held here with
    the same content takes from[] from the record; the preview's heading is
    the Studio's, the name the link's quoted claim; a link arriving over
    another sheet closes it; #P= opens as #p=. (A CJK name and 300 colours
    through the Studio's own Share went with Share: sharing is shelved.)"""
    page = s_page(browser, base, site, errors)
    one = [[1 if i % 7 == 0 else 0 for i in range(4096)]]

    def part(what, fn):
        nonlocal page
        try:
            fn()
        except Exception as e:  # (one finding that can't finish fails alone; the next on a fresh page)
            check(False, f"/studio/ links review: {what} did not finish: {str(e).splitlines()[0][:120]}")
            page.context.close()
            page = s_page(browser, base, site, errors)

    def timers():
        frames = [[1 if (i % 128) == f * 10 else 0 for i in range(4096)] for f in range(4)]
        code = link_p(frames, [20] * 4, [0xf7a531])
        page.goto("about:blank")
        page.goto(f"{base}/studio/")
        page.wait_for_selector(S_READY, timeout=30000)
        page.evaluate("""() => { window.__paints = 0; const p = CanvasRenderingContext2D.prototype.putImageData;
            CanvasRenderingContext2D.prototype.putImageData = function (...a) { if (this.canvas.id !== 's-canvas') __paints++; return p.apply(this, a); }; }""")
        for _ in range(30):
            page.evaluate("(c) => { location.hash = '#p=' + c; }", code)
            wait_js(page, f"{L_SHOWN} === 'shown'", timeout=10000)
            page.evaluate(f"delete {L_SHOWN.replace('.linkOpened', '.linkOpened')}")
        page.wait_for_timeout(500)
        page.evaluate("window.__paints = 0")
        page.wait_for_timeout(1000)
        live = page.evaluate("window.__paints")
        page.click("#s-lk button:has-text('Close')")
        page.wait_for_timeout(300)
        page.evaluate("window.__paints = 0")
        page.wait_for_timeout(1000)
        after = page.evaluate("window.__paints")
        check(live <= 60 and after == 0, f"/studio/ links review: 30 links in a row play as one (about 50 paints a second: {live}), and nothing paints after Close ({after})")

    def bidi():
        rlo = "‮evil​"
        code = link_p(one, [100], [0xf7a531], name="Nice⁦ one", author=rlo)
        said = l_goto(page, base, "#p=" + code)
        who = page.inner_text("#s-lk-who")
        bdis = page.evaluate("document.querySelectorAll('#s-lk bdi').length")
        page.click("#s-lk-open")
        wait_js(page, f"{L_SHOWN} === 'opened'", timeout=20000)
        s_kept(page)
        p = page.evaluate(I_PIECE)
        e = (p["from"] or [{}])[0]
        clean = lambda t: not re.search("[؜​-‏‪-‮⁠-⁩﻿]", t or "")
        check(said == "shown" and "by “evil”" in who and bdis == 2 and p["name"] == "Nice one" and clean(e.get("title")) and clean(e.get("author")),
              f"/studio/ links review: a link's bidi and zero-width controls dropped, shown quoted and isolated, stored clean ('{who[:40]}', {p['name']!r}, {e})")
        f = kev(page, "const r = await K.stamp('Pad', 4, 4, { env: { author: '\\u202e' + 'gninraw' } }); return K.file(r, [r]);")
        file_open(page, f, "pad.dmdart")
        claim = page.inner_text("#s-fs .s-fs-claim")
        has_bdi = page.evaluate("!!document.querySelector('#s-fs .s-fs-claim bdi')")
        file_add(page)
        stored = page.evaluate("""async () => { const m = await import('/studio/store.js'); const s = await m.LocalStore.open(); const l = (await s.list({ type: 'stamp' })).items; s.db.close(); return l.map((h) => h.author.name); }""")
        check(has_bdi and "by “gninraw”" in claim and stored == ["gninraw"], f"/studio/ links review: a file's author too: quoted, isolated, stored without its controls ('{claim[:50]}', {stored})")

    def held():
        page.mouse.click(*s_at(page, 6, 6))
        s_kept(page)
        page.fill("#s-name", "My own")
        s_kept(page)
        r = page.evaluate(S_STORE, ["get", None])["r"]
        code = link_p(one, [100], [0xf7a531], name="Someone else's", author="liar", src=(r["id"], 99, r["hash"]))
        l_goto(page, base, "#p=" + code)
        h2, nm = page.inner_text("#s-lk h2"), page.inner_text("#s-lk-name")
        page.click("#s-lk-open")
        wait_js(page, f"{L_SHOWN} === 'opened'", timeout=20000)
        s_kept(page)
        e = (page.evaluate(I_PIECE)["from"] or [{}])[0]
        check(e.get("id") == r["id"] and e.get("name") == "My own" and e.get("rev") == r["rev"] and e.get("author", {}).get("name") != "liar",
              f"/studio/ links review: a link naming my record with its content: from[] is my record's, not the link's ({e.get('name')}, rev {e.get('rev')})")
        check(h2 == "A shared piece" and nm == "Called “Someone else's”", f"/studio/ links review: the heading is the Studio's, the name the link's quoted claim ({h2!r}, {nm!r})")

    def over():
        code = link_p(one, [100], [0xf7a531], name="Over")
        page.goto("about:blank")
        page.goto(f"{base}/studio/")
        page.wait_for_selector(S_READY, timeout=30000)
        page.click("#s-send")
        page.wait_for_selector("#s-p[open]", timeout=20000)
        page.evaluate("(c) => { location.hash = '#p=' + c; }", code)
        wait_js(page, f"{L_SHOWN} === 'shown'", timeout=10000)
        check(not page.is_visible("#s-p") and page.is_visible("#s-lk"), "/studio/ links review: a link arriving over another sheet closes it and shows")
        said = l_goto(page, base, "#P=" + code)
        check(said == "shown", f"/studio/ links review: #P= opens as #p= ({said})")

    for what, fn in (("timers", timers), ("bidi", bidi), ("a held record", held), ("another sheet", over)):
        part(what, fn)
    page.context.close()


S_TIDY_REC = """async (id) => { const m = await import('/studio/store.js'); const s = await m.LocalStore.open(); const r = await s.get(id); s.db.close(); return r && { name: r.name, note: r.note ?? null, tags: r.tags, author: r.author, from: r.from, keys: Object.keys(r) }; }"""


def studio_tidy(browser, base: str, site: Path, errors: list[str]) -> None:
    """The studio-tidy follow-ups. plain(): the joiners (ZWNJ, ZWJ) kept
    between characters (Persian, an emoji family) and dropped at the ends,
    doubled or beside a space; bidi controls, zero-width spaces, controls,
    soft hyphens, tag characters outside a flag and lone surrogates dropped,
    from a file's names, authors, notes, tags and every from[] entry, from a
    link's and from Import's fields; a note left empty goes. The Library's
    Made from quotes and isolates each claim. A split backup's part opened
    without the rest says so and offers Pick all; numbers that don't add up,
    or a pack not tagged backup, say nothing. A pack naming records neither
    the file nor this browser holds says so on its row, and the Library's
    Holds counts them. Framed /create/: a drop the converter took is left to
    it; one outside takes a picture or an https address, never another file."""
    page = s_page(browser, base, site, errors)
    ZW, ZN = "‍", "‌"
    fa = f"\U0001F468{ZW}\U0001F469{ZW}\U0001F467"  # a family: three people joined
    fa_name = f"{ZW}{ZW}{fa}{ZW}{ZN}"
    persian = f"می{ZN}خواهم"
    flag = "\U0001F3F4\U000E0067\U000E0062\U000E0065\U000E006E\U000E0067\U000E007F"

    def part(what, fn):
        nonlocal page
        try:
            fn()
        except Exception as e:  # (one finding that can't finish fails alone)
            check(False, f"/studio/ tidy: {what} did not finish: {str(e).splitlines()[0][:120]}")
        page.context.close()  # (each on a fresh page)
        page = s_page(browser, base, site, errors)

    def joiners():
        got = kev(page, """const [persian, fa, flag] = A;
            const r = await K.stamp(persian + '\\u200d', 4, 4, { env: { author: fa, tags: ['x\\u200d', '\\u200b', 'ok\\u00ad'] },
              set: { note: 'a\\u200d\\u200db \\u202eevil\\u202c ' + flag + ' hi\\u{e0041}\\u{e0042}', from: [{ external: true, note: '\\u2066Fake\\u2069\\u200b' }, { external: true, note: '\\u202e' }, { id: 'st_' + '0'.repeat(32), rev: 1, hash: 'sha256-' + '0'.repeat(64), name: '\\u2067Mal\\u200dlory\\u200d', author: { name: '\\u202eyrollam', account: null }, title: '\\u061ct', platform: 'p\\u200f' }] } });
            const e = await K.stamp('Empty note', 4, 4, { set: { note: '\\u200b\\u202e\\u200d' } });
            const pk = await K.pack('Two', [r, e]);
            return { id: r.id, e: e.id, file: K.file(pk, [pk, r, e]) };""", [persian, fa, flag])
        said = file_open(page, got["file"], "joiners.dmdpack")
        claims = page.eval_on_selector_all("#s-fs .s-fs-claim bdi", "els => els.map((e) => e.textContent)")
        file_add(page)
        r, e = page.evaluate(S_TIDY_REC, got["id"]), page.evaluate(S_TIDY_REC, got["e"])
        f = r["from"]
        check(said == "asked" and r["name"] == persian and r["author"]["name"] == fa and fa in claims,
              f"/studio/ tidy: a file's joiners kept between characters: Persian's ZWNJ and an emoji family's ZWJs ({r['name']!r}, {r['author']['name']!r}), a trailing one dropped")
        check(r["note"] == f"a{ZW}b evil {flag} hi" and r["tags"] == ["x", "ok"] and e["note"] is None and "note" not in e["keys"],
              f"/studio/ tidy: a note and tags as plain text: a doubled joiner once, bidi, tag characters (a flag's kept) and soft hyphens gone, an empty tag gone, a note left empty gone ({r['note']!r}, {r['tags']}, {e['note']!r})")
        check(f[0]["note"] == "Fake" and f[1]["note"] == "" and f[2]["name"] == f"Mal{ZW}lory" and f[2]["author"]["name"] == "yrollam" and f[2]["title"] == "t" and f[2]["platform"] == "p",
              f"/studio/ tidy: every from[] entry's note, name, author, title and platform plain ({f})")
        # The Library's Made from: each claim quoted and isolated.
        lib_open(page)
        lib_kind(page, "stamp")
        lib_detail(page, persian)
        dd = page.evaluate("""() => { const dt = [...document.querySelectorAll('#s-ldet dt')].find((x) => x.textContent === 'Made from'); const dd = dt.nextElementSibling;
            return { text: dd.textContent, bdi: [...dd.querySelectorAll('bdi')].map((b) => b.textContent) }; }""")
        check(dd["bdi"] == ["Fake", f"Mal{ZW}lory"] and dd["text"].startswith("“Fake”, an imported picture, “Mal") and "(version 1)" in dd["text"],
              f"/studio/ tidy: the Library's Made from quotes and isolates each claim, a note left empty said as an imported picture ({dd})")

    def hostile_names():
        # 4,000 records, each named to abuse: joiners by the hundred, every control at once, a lone surrogate (JSON's \\ud800).
        raw = kev(page, """const recs = []; const a = new Uint8Array(16).fill(1); const nasty = '\\u200d'.repeat(100) + '\\u202e\\u2066\\u200b\\u0000\\u0007\\u2028\\ufeff\\u{e0001}';
            for (let i = 0; i < 4000; i++) { const r = m.record('stamp', (i % 2 ? nasty : '\\u200c\\u200d'.repeat(50)), { w: 4, h: 4, colours: [{ k: 1, hex: '#ff0000' }], frames: [{ px: await m.packPx(a) }] }, { author: nasty }); r.note = nasty + 'n'; r.hash = await m.hashOf(r.body); recs.push(r); }
            const pk = await K.pack('Many', recs); return K.file(pk, [pk, ...recs]).replace('"Many"', '"M\\\\ud800any"');""")
        said = file_open(page, raw, "nasty.dmdpack")
        bad = page.evaluate("""() => [...document.querySelectorAll('#s-fs li')].filter((li) => /[\\u0000-\\u001f\\u061c\\u200b\\u200e\\u200f\\u2028-\\u202e\\u2060-\\u2069\\ufeff\\u{e0000}-\\u{e007f}]|[\\u200c\\u200d]{2}|^[\\u200c\\u200d]/u.test(li.querySelector('b').textContent + li.querySelector('bdi')?.textContent)).length""")
        n = file_add(page)
        stored = page.evaluate("""async () => { const m = await import('/studio/store.js'); const s = await m.LocalStore.open(); const l = (await s.list({ type: 'stamp' })).items; s.db.close();
            return { n: l.length, names: [...new Set(l.map((h) => h.name))], authors: [...new Set(l.map((h) => h.author.name))], notes: [...new Set(l.map((h) => h.note ?? null))] }; }""")
        check(said == "asked" and bad == 0 and n == 4001 and stored["names"] == [""] and stored["authors"] == [""] and stored["notes"] == ["n"],
              f"/studio/ tidy: 4,000 records named in joiners and controls open, shown and stored as plain text, a lone surrogate dropped ({n} added; {stored['names']}, {stored['notes']})")

    def link_joiners():
        one = [[1 if i % 9 == 0 else 0 for i in range(4096)]]
        code = link_p(one, [100], [0xf7a531], name=f"{ZW}{persian}{ZW}{ZW}", author=fa_name)
        said = l_goto(page, base, "#p=" + code)
        nm, who = page.inner_text("#s-lk-name"), page.inner_text("#s-lk-who")
        page.click("#s-lk-open")
        wait_js(page, f"{L_SHOWN} === 'opened'", timeout=20000)
        s_kept(page)
        p = page.evaluate(I_PIECE)
        e = (p["from"] or [{}])[0]
        prov = page.inner_text("#s-prov")
        check(said == "shown" and p["name"] == persian and e.get("author") == fa and f"“{fa}”" in who and persian in nm,
              f"/studio/ tidy: a link's joiners kept between characters, dropped at the ends and doubled ({p['name']!r}, {e.get('author')!r})")
        check("Made from “⁨A shared link⁩”" in prov, f"/studio/ tidy: the piece's Made from line quotes and isolates the claim ({prov[-60:]!r})")

    def parts():
        got = kev(page, """const P = await K.palette('Part colours'); const S1 = await K.stamp('Part stamp'); const S2 = await K.stamp('Other stamp');
            const day = 'Backup, 2026-11-12';
            const b1 = await K.pack(day + ' (1 of 3)', [P], { env: { tags: ['backup'] } }), b2 = await K.pack(day + ' (2 of 3)', [S1], { env: { tags: ['backup'] } }), b3 = await K.pack(day + ' (3 of 3)', [S2], { env: { tags: ['backup'] } });
            const odd = await K.pack(day + ' (5 of 3)', [S1], { env: { tags: ['backup'] } }), untagged = await K.pack(day + ' (1 of 2)', [S1]);
            return { b1: K.file(b1, [b1, P]), b2: K.file(b2, [b2, S1]), b3: K.file(b3, [b3, S2]), odd: K.file(odd, [odd, S1]), untagged: K.file(untagged, [untagged, S1]) };""")
        def sheet(files):
            page.evaluate("delete document.getElementById('studio').dataset.opened")
            with page.expect_file_chooser(timeout=20000) as fc:
                page.click("#s-openf")
            fc.value.set_files([{"name": n, "mimeType": "application/json", "buffer": got[n].encode()} for n in files])
            wait_js(page, "['asked', 'refused'].includes(document.getElementById('studio').dataset.opened)", timeout=60000)
            return {"said": page.evaluate("document.getElementById('s-fs-parts')?.textContent || ''"),
                    "pick": page.evaluate("[...document.querySelectorAll('#s-fs[open] button')].map((b) => b.textContent)")}
        one = sheet(["b2"])
        with page.expect_file_chooser(timeout=10000) as fc:
            page.click("#s-fs[open] button:has-text('Pick all 3')")
        chooser = fc.value.is_multiple()
        closed = not page.is_visible("#s-fs")
        two = sheet(["b1", "b3"])
        page.click("#s-fs[open] button:has-text('Cancel')")
        three = sheet(["b1", "b2", "b3"])
        page.click("#s-fs[open] button:has-text('Cancel')")
        odd = sheet(["odd"])
        page.click("#s-fs[open] button:has-text('Cancel')")
        untagged = sheet(["untagged"])
        page.click("#s-fs[open] button:has-text('Cancel')")
        check(one["said"].startswith("This is one part of a backup in 3 files") and "Pick all 3" in one["pick"] and chooser and closed,
              f"/studio/ tidy: one part of a split backup picked alone says so and offers Pick all 3, which opens the picker for several ({one['said'][:60]!r}, {one['pick']})")
        check(two["said"].startswith("These are 2 parts of a backup in 3 files") and not three["said"] and "Pick all 3" not in three["pick"],
              f"/studio/ tidy: two of three parts say so; all three picked together say nothing ({two['said'][:40]!r}, {three['said']!r})")
        check(not odd["said"] and not untagged["said"], f"/studio/ tidy: a part 5 of 3, or a pack not tagged backup, says nothing of parts ({odd['said']!r}, {untagged['said']!r})")
        # The same part as a shared link (#k=): a link is no file the user picked, so no Pick all.
        import base64
        import zlib
        code = base64.urlsafe_b64encode(zlib.compress(got["b2"].encode())).decode().rstrip("=")
        said = l_goto(page, base, "#k=" + code)
        lk = page.evaluate("document.getElementById('s-fs-parts')?.textContent || ''")
        check(said == "pack" and not lk, f"/studio/ tidy: a backup part as a #k= link offers no Pick all ({said}, {lk!r})")

    def dangling():
        got = kev(page, """const st = await K.stamp('Here'); const den = (await import('/studio/palettes.js')); await den.ready; const sid = den.STARTER[0].id;
            const absent = [m.newId('stamp'), m.newId('stamp')];
            const pk = m.record('pack', 'Names absent ones', { members: [{ id: st.id, rev: st.rev }, { id: sid, rev: den.STARTER[0].rev }, ...absent.map((id) => ({ id, rev: 1 }))] }); pk.hash = await m.hashOf(pk.body);
            const ok = m.record('pack', 'Names held ones', { members: [{ id: st.id, rev: st.rev }] }); ok.hash = await m.hashOf(ok.body);
            const root = await K.pack('Root', [st, pk, ok]); return K.file(root, [root, st, pk, ok]);""")
        said = file_open(page, got, "absent.dmdpack")
        rows = page.evaluate("""() => Object.fromEntries([...document.querySelectorAll('#s-fs li')].map((li) => [li.querySelector('b').textContent, li.querySelector('[data-missing]')?.textContent || '']))""")
        file_add(page)
        lib_open(page)
        lib_kind(page, "pack")
        lib_detail(page, "Names absent ones")
        holds = page.evaluate("""() => { const dt = [...document.querySelectorAll('#s-ldet dt')].find((x) => x.textContent === 'Holds'); const dd = dt?.nextElementSibling; return dd ? { text: dd.textContent, missing: dd.querySelector('[data-missing]')?.dataset.missing } : null; }""")
        check(said == "asked" and rows.get("Names absent ones") == " · names 2 records not in the file or this browser" and rows.get("Names held ones") == "" and rows.get("Here") == "",
              f"/studio/ tidy: a pack naming records neither the file nor this browser holds says how many on its row (a starter record not among them) {rows}")
        check(holds and holds["missing"] == "2" and holds["text"].startswith("Here, ") and holds["text"].endswith(" · 2 not in this browser"),
              f"/studio/ tidy: the Library's Holds names a pack's members and says the ones not here ({holds})")

    def drops():
        nonlocal page
        page.context.close()
        page = i_page(browser, base, site, errors)
        fr = i_open(page)
        png, gif = c_fixtures()
        import base64
        b64 = base64.b64encode(gif).decode()
        drop = """([b64, where, kind]) => { const dt = new DataTransfer();
            if (kind === 'gif') dt.items.add(new File([Uint8Array.from(atob(b64), (c) => c.charCodeAt(0))], 'w.gif', { type: 'image/gif' }));
            if (kind === 'text') dt.items.add(new File(['not a picture'], 'notes.txt', { type: 'text/plain' }));
            if (kind === 'url') dt.setData('text/uri-list', 'https://raw.githubusercontent.com/x/y/z.png');
            const e = new DragEvent('drop', { dataTransfer: dt, bubbles: true, cancelable: true }); document.querySelector(where).dispatchEvent(e); return e.defaultPrevented; }"""
        fr.evaluate("document.querySelector('#st-msg').textContent = ''")
        p_text = fr.evaluate(drop, [b64, "#c-cap", "text"])
        fr.wait_for_timeout(500)
        msg_text = fr.inner_text("#st-msg")
        p_in = fr.evaluate(drop, [b64, "#studio .st-grid", "gif"])
        wait_js(fr, "document.querySelector('#st-msg').textContent.startsWith('Dropped: 88')", timeout=20000)
        fr.wait_for_timeout(500)
        msg_in = fr.inner_text("#st-msg")
        p_url = fr.evaluate(drop, [b64, "#c-cap", "url"])
        fr.wait_for_timeout(300)
        url = fr.input_value("#st-url")
        check(p_text and not msg_text.startswith("Your file"), f"/create/ framed: a file that is no picture dropped outside the converter is kept from the browser and not loaded ({msg_text!r})")
        check(p_in and msg_in.startswith("Dropped: 88"), f"/create/ framed: a drop the converter took is its own, not taken again by the page ({msg_in[:30]!r})")
        check(p_url and url == "https://raw.githubusercontent.com/x/y/z.png", f"/create/ framed: an address dropped outside the converter is fetched as the converter would ({url!r})")

    def import_names():
        nonlocal page
        page.context.close()
        page = i_page(browser, base, site, errors)
        fr = i_open(page)
        png, gif = c_fixtures()
        i_file(fr, "w.gif", gif, "image/gif")
        wait_js(fr, "document.querySelector('#studio').dataset.animated === '1'", timeout=20000)
        fr.fill("#st-title", f"‮{persian}​{ZW}")
        fr.fill("#st-author", fa_name)
        got = i_use(page, fr)
        p = page.evaluate(I_PIECE)
        e = (p["from"] or [{}])[0]
        check(got.startswith("piece") and p["name"] == persian and e.get("title") == persian and e.get("author") == fa,
              f"/studio/ tidy: Import's title and author as plain text, the joiners between characters kept ({p['name']!r}, {e.get('title')!r}, {e.get('author')!r})")

    for what, fn in (("joiners", joiners), ("hostile names", hostile_names), ("a link's joiners", link_joiners), ("backup parts", parts),
                     ("absent members", dangling), ("framed drops", drops), ("Import's names", import_names)):
        part(what, fn)
    page.context.close()


def i_colour_fixtures() -> dict:
    """128x32 pictures past 256 colours: a still of 4,096 distinct colours (gradients, among them
    dark ones that are not black, and some pure black), and a 3-frame GIF whose frames have 200
    colours each, 600 in all, with its own times."""
    still = Image.new("RGB", (128, 32))
    px = []
    for y in range(32):
        for x in range(128):
            if x < 4:
                px.append((0, 0, 0))  # pure black: off
            elif x < 12:
                px.append((x - 3, y // 4, (x + y) % 3))  # dark, not black (1..8 on red)
            else:
                px.append(((x * 2) & 255, (y * 8) & 255, ((x * 7 + y * 3) & 255)))
    still.putdata(px)
    b = io.BytesIO()
    still.save(b, "PNG")
    frames = []
    for f in range(3):
        im = Image.new("P", (128, 32))
        pal = []
        for i in range(256):
            c = f * 200 + i
            pal += [40 + (c % 200), (c * 3) % 256, 10 + (c // 3) % 240] if i < 200 else [0, 0, 0]
        im.putpalette(pal)
        im.putdata([(x + y * 7) % 200 for y in range(32) for x in range(128)])
        frames.append(im)
    g = io.BytesIO()
    frames[0].save(g, "GIF", save_all=True, append_images=frames[1:], duration=[80, 120, 160], loop=0, optimize=False, disposal=1)
    return {"still": b.getvalue(), "still_px": px, "anim": g.getvalue()}


def studio_import_colours(browser, base: str, site: Path, errors: list[str]) -> None:
    """Import keeps the picture's every colour (the panel's 24 bits): a still
    of 4,096 colours and an animated GIF of 600 come in pixel for pixel, pure
    black as off, dark colours that are not black kept as they are, the
    piece past 255 colours in two-byte keys and said so; "Fit to 256
    colours" still gives the device converter's 256. /create/'s own Download
    GIF is unchanged (256 at most) and has no Fit box."""
    fx = i_colour_fixtures()
    page = i_page(browser, base, site, errors)
    fr = i_open(page)
    i_file(fr, "many.png", fx["still"], "image/png")
    wait_js(fr, "document.querySelector('#st-msg').textContent.startsWith('Picture 128')", timeout=20000)
    got = i_use(page, fr)
    p = page.evaluate(I_PIECE)
    want = [(r << 16 | g << 8 | b) for r, g, b in fx["still_px"]]
    have = i_frames(p)[0] if p else []
    wrong = [i for i, (a, b) in enumerate(zip(have, want)) if a != b]
    dark = [have[y * 128 + x] for y in range(32) for x in range(4, 12)] if have else []
    check(got.startswith("piece") and not wrong and len(have) == 4096 and p["colours"] == len(set(want) - {0}) and all(dark),
          f"/studio/ import colours: a still of {len(set(want))} colours comes in pixel for pixel, black as off and dark colours kept ({got}, {len(wrong)} pixels wrong, "
          f"{p['colours'] if p else 0} colours; first wrong {[(i, hex(have[i]), hex(want[i])) for i in wrong[:3]]})")
    page.context.close()
    page = i_page(browser, base, site, errors)
    fr = i_open(page)
    i_file(fr, "many.gif", fx["anim"], "image/gif")
    wait_js(fr, "document.querySelector('#studio').dataset.animated === '1'", timeout=20000)
    got = i_use(page, fr)
    p = page.evaluate(I_PIECE)
    im = Image.open(io.BytesIO(fx["anim"]))
    want = []
    for i in range(im.n_frames):
        im.seek(i)
        want.append([(r << 16 | g << 8 | b) for r, g, b in im.convert("RGB").getdata()])
    have = i_frames(p) if p else []
    wrong = sum(1 for a, b in zip(have, want) for x, y in zip(a, b) if x != y)
    check(got.startswith("piece") and len(have) == 3 and not wrong and p["ms"] == [80, 120, 160] and p["colours"] == 600,
          f"/studio/ import colours: an animated GIF of 600 colours comes in pixel for pixel with its times ({got}, {len(have)} frames, {wrong} pixels wrong, {p['colours'] if p else 0} colours, {p['ms'] if p else None})")
    page.context.close()
    # Fit to 256 colours, ticked: the converter's own GIF, as before (256 at most).
    page = i_page(browser, base, site, errors)
    fr = i_open(page)
    i_file(fr, "many.png", fx["still"], "image/png")
    wait_js(fr, "document.querySelector('#st-msg').textContent.startsWith('Picture 128')", timeout=20000)
    fitted = fr.is_visible("#c-fit") and not fr.is_checked("#c-fit")
    fr.check("#c-fit")
    got = i_use(page, fr)
    p = page.evaluate(I_PIECE)
    check(fitted and got.startswith("piece") and p and p["colours"] <= 256, f"/studio/ import colours: Fit to 256 colours, offered unticked, ticked gives the converter's 256 ({got}, {p['colours'] if p else 0})")
    page.context.close()
    # /create/ itself: Download GIF still writes 256 colours at most, and no Fit box.
    plain = new_page(browser, "laptop", errors)
    i_route(plain, site, base)
    plain.goto(f"{base}/create/")
    plain.wait_for_selector("#create:not([aria-busy]) #studio .st-grid", timeout=30000)
    i_file(plain, "many.png", fx["still"], "image/png")
    wait_js(plain, "document.querySelector('#st-msg').textContent.startsWith('Picture 128')", timeout=20000)
    plain.fill("#st-title", "Many")
    wait_js(plain, "!!document.querySelector('#studio').dataset.name", timeout=5000)
    im = c_download(plain)
    n = len(im.convert("RGB").getcolors(1 << 16) or [])
    check(n <= 256 and plain.query_selector("#c-fit") is None, f"/create/: Download GIF of a picture of 3,905 colours still writes 256 at most ({n}), and offers no Fit box")
    plain.context.close()


def studio_tidy_review(browser, base: str, site: Path, errors: list[str]) -> None:
    """The studio-tidy review: an animation past the 4,096 colours a piece
    holds is fitted to 256 by itself and says so (never refused); a file's
    from[] entries whose words aren't text, whose author is neither text nor
    {name, account}, whose rev isn't a whole number or whose id isn't a
    record id are refused; blank-looking fillers, the grapheme joiner,
    deprecated format controls and tag characters smuggled after a flag
    come out of names."""
    # 20 frames of 256 colours each, none shared: 5,120 in all.
    frames = []
    for f in range(20):
        im = Image.new("P", (128, 32))
        pal = []
        for i in range(256):
            c = f * 256 + i
            pal += [16 + (c % 240), 16 + (c * 7) % 240, 16 + (c // 22) % 240]
        im.putpalette(pal)
        im.putdata([(x + y * 5 + f) % 256 for y in range(32) for x in range(128)])
        frames.append(im)
    g = io.BytesIO()
    frames[0].save(g, "GIF", save_all=True, append_images=frames[1:], duration=60, loop=0, optimize=False, disposal=1)
    page = i_page(browser, base, site, errors)
    fr = i_open(page)
    i_file(fr, "photo.gif", g.getvalue(), "image/gif")
    wait_js(fr, "document.querySelector('#studio').dataset.animated === '1'", timeout=20000)
    got = i_use(page, fr)
    p = page.evaluate(I_PIECE)
    said = page.inner_text("#s-err")
    check(got == "piece:20" and p and p["colours"] <= 256 and "more than 4,096 colours, so it was fitted to 256" in said,
          f"/studio/ tidy review: an animation of 5,120 colours is fitted to 256 by itself and says so ({got}, {p['colours'] if p else 0} colours, '{said[:70]}')")
    page.context.close()
    page = s_page(browser, base, site, errors)
    bad = {
        "a note that is an array": "{ external: true, note: ['\\u2069\\u202eevil'] }",
        "a name that is an object": "{ id: 'st_' + '0'.repeat(32), rev: 1, hash: 'sha256-' + '0'.repeat(64), name: { toString: 1 } }",
        "a rev that is text": "{ id: 'st_' + '0'.repeat(32), rev: '1\\u202e) fake', hash: 'sha256-' + '0'.repeat(64), name: 'x' }",
        "an id that isn't one": "{ id: 'pc_\\u202e', rev: 1, hash: 'sha256-' + '0'.repeat(64), name: 'x' }",
        "an author with no name": "{ external: true, note: 'n', author: { name: 5 } }",
        "a platform that is a number": "{ external: true, note: 'n', platform: 9 }",
    }
    out = {}
    for what, entry in bad.items():
        f = kev(page, "const r = await K.stamp('Pad', 4, 4, { set: { from: [" + entry + "] } }); return K.file(r, [r]);")
        out[what] = file_open(page, f, "bad-from.dmdart")
        if out[what] == "asked":
            page.click("#s-fs[open] button:has-text('Cancel')")
    ok_entry = kev(page, "const r = await K.stamp('Fine', 4, 4, { set: { from: [{ external: true, note: 'n', title: 't', platform: 'SNES', author: 'a' }, { id: 'st_' + '0'.repeat(32), rev: 0, hash: 'sha256-' + '0'.repeat(64), name: 'x', author: { name: 'y', account: null } }] } }); return K.file(r, [r]);")
    fine = file_open(page, ok_entry, "fine.dmdart")
    check(all(v == "refused" for v in out.values()) and fine == "asked", f"/studio/ tidy review: from[] entries that aren't text where text goes are refused {out}; a right one opens ({fine})")
    if fine == "asked":
        page.click("#s-fs[open] button:has-text('Cancel')")
    names = kev(page, """const m2 = await import('/studio/store.js'); const p = m2.plain;
        return [p('\\u3164\\u3164'), p('\\u2800'), p('\\u115f\\u1160\\uffa0'), p('a\\u034fb\\u206ac'), p('\\u{1f3f4}' + [...'say hello'].map((c) => String.fromCodePoint(0xe0000 + c.charCodeAt(0))).join('') + '\\u{e007f}'),
          p('\\u{1f3f4}\\u{e0067}\\u{e0062}\\u{e0073}\\u{e0063}\\u{e0074}\\u{e007f}')];""")
    check(names == ["", "", "", "abc", "\U0001F3F4", "\U0001F3F4\U000E0067\U000E0062\U000E0073\U000E0063\U000E0074\U000E007F"],
          f"/studio/ tidy review: blank fillers, the grapheme joiner, deprecated controls and a sentence smuggled after a flag dropped; Scotland's flag kept {names!r}")
    page.context.close()


def studio_budget(site: Path, web_size: Path = ROOT / "tools/web-size.py") -> None:
    """web-size fails on a file in studio/ that is on no budget line."""
    with tempfile.TemporaryDirectory() as t:
        share = Path(t) / "share"
        shutil.copytree(ROOT / "opendmd-web/share", share)
        clean = subprocess.run(["uv", "run", str(web_size), str(ROOT / "opendmd-web/www-dist"), "--share", str(share)], capture_output=True, text=True)
        (share / "studio/stray.js").write_text("export const x = 1;\n")
        stray = subprocess.run(["uv", "run", str(web_size), str(ROOT / "opendmd-web/www-dist"), "--share", str(share)], capture_output=True, text=True)
    check(clean.returncode == 0 and stray.returncode != 0 and "UNLISTED studio/stray.js" in stray.stdout,
          f"/studio/ budget: web-size passes the tree and fails a stray studio/stray.js ({clean.returncode}, {stray.returncode})")


def create_page(browser, base: str, site: Path, errors: list[str], shots: Path | None) -> None:
    png, gif = c_fixtures()
    for size in SIZES:
        page = new_page(browser, size, errors)
        page.add_init_script("""document.addEventListener('securitypolicyviolation', e =>
            (window.__csp = window.__csp || []).push(e.violatedDirective + ' ' + e.blockedURI))""")
        page.route(lambda u: urllib.parse.urlsplit(u).path.startswith("/create/") and urllib.parse.urlsplit(u).path.endswith(("/", ".html")),
                   lambda r: (lambda resp: r.fulfill(response=resp, headers={**resp.headers, "content-security-policy": csp_for(site, "/create/*")}))(r.fetch()))
        asked: list[str] = []
        c_stub(page, base, png, gif, asked)
        page.goto(f"{base}/create/?q=studio+test&platform=snes")
        page.wait_for_selector("#create:not([aria-busy]) #studio .st-grid", timeout=30000)
        page.wait_for_selector("#find-grid .find-hit", timeout=20000)
        tiles = page.eval_on_selector_all("#find-grid .find-hit", "els => els.map(e => e.dataset.kind)")
        src = page.eval_on_selector_all("#st-src button", "els => els.map(e => [e.dataset.v, e.classList.contains('on')])")
        check(tiles == ["Named_Titles", "Named_Snaps", "Named_Snaps"] and page.input_value("#st-q") == "studio test"
              and page.input_value("#find-platform") == "SNES" and src == [["gameart", True], ["gifcities", False], ["own", False]],
              f"/create/ ({size}): ?q=&platform= searches Game art on that platform at once ({tiles}, {src})")
        check(page.is_hidden("#c-cap") and page.inner_text("h1") == "Create Art" and page.locator("#st-save").inner_text() == "Download GIF"
              and page.locator("#st-cat-l").is_hidden(), f"/create/ ({size}): the Studio, Save downloads (no category: there is no card)")
        page.click("#find-grid .find-hit[data-kind=Named_Titles]")
        wait_js(page, "document.querySelector('#studio').dataset.title === 'Studio Test'", timeout=20000)
        d = c_data(page)
        check(d.get("platform") == "SNES" and d.get("name") == "SNES_Studio_Test_01.gif" and page.is_visible("#st-frame") and page.is_hidden("#find-grid"),
              f"/create/ ({size}): a picture loaded, its platform and title, Find folded ({d.get('platform')}, {d.get('name')})")
        if size == "phone":
            check(page.evaluate("document.documentElement.scrollWidth <= innerWidth"), "/create/ (phone): no sideways scroll with a picture in")
            if shots:
                page.screenshot(path=str(shots / "create-phone.png"), full_page=True)
            check(not page.evaluate("window.__csp || []"), f"/create/ ({size}): no CSP violation {page.evaluate('window.__csp || []')[:2]}")
            page.context.close()
            continue
        # Edits: text, colours.
        page.fill("#st-text", "HI")
        page.select_option("#st-colours", "16")
        wait_js(page, "document.querySelector('#studio').dataset.text === '1'", timeout=5000)
        page.wait_for_timeout(400)
        if shots:
            page.evaluate("scrollTo(0, 0)")
            page.screenshot(path=str(shots / "create-laptop-still.png"), full_page=True)
        im = c_download(page)
        n = getattr(im, "n_frames", 1)
        im.seek(0)
        colours = len(im.convert("RGB").getcolors(4096) or [])
        check(im.format == "GIF" and im.size == (128, 32) and im.filename_suggested == "SNES_Studio_Test_01.gif" and 1 < colours <= 17,
              f"/create/: Download GIF -> {im.filename_suggested}, {im.size}, {n} frame(s), {colours} colours")
        # GifCities: an animated GIF, trimmed and reversed.
        page.click("#st-change")
        page.click("#st-src [data-v=gifcities]")
        page.fill("#st-q", "mario")
        page.click("#st-go")
        wait_js(page, "document.querySelectorAll('#find-grid .find-hit').length === 2", timeout=20000)
        page.click("#find-grid .find-hit[data-id=WIDE]")
        wait_js(page, "document.querySelector('#studio').dataset.animated === '1'", timeout=20000)
        d = c_data(page)
        check(d.get("srcFrames") == "4" and page.is_visible("#st-tl") and "Seen on" in page.inner_text("#st-import"),
              f"/create/: GifCities -> an animated GIF on the timeline ({d.get('srcFrames')} frames)")
        page.click("#st-tl .st-cell >> nth=3")
        page.click("#st-del")
        page.check("#st-rev")
        wait_js(page, "document.querySelector('#studio').dataset.frames === '3'", timeout=5000)
        page.fill("#st-title", "Mario banner")
        wait_js(page, "document.querySelector('#studio').dataset.name === 'SNES_Mario_banner_01.gif'", timeout=5000)
        if shots:
            page.evaluate("scrollTo(0, 0)")
            page.screenshot(path=str(shots / "create-laptop-anim.png"), full_page=True)
        im = c_download(page)
        xs = []  # where the yellow box starts in each frame: it moved right, so reversed it moves left
        for i in range(im.n_frames):
            im.seek(i)
            rgb = im.convert("RGB")
            xs.append(min((x for x in range(128) for y in range(32) if (lambda p: p[0] > 180 and p[1] > 140 and p[2] < 90)(rgb.getpixel((x, y)))), default=-1))
        check(im.size == (128, 32) and im.n_frames == 3 and im.filename_suggested == "SNES_Mario_banner_01.gif" and xs[0] > xs[1] > xs[2] >= 0,
              f"/create/: the animation downloaded, the last frame deleted, reversed ({im.filename_suggested}, {im.n_frames} frames, box at {xs})")
        # Your own picture.
        page.click("#st-change")
        page.click("#st-src [data-v=own]")
        with tempfile.TemporaryDirectory() as t:  # (the browser reads the file from its path: kept until it has)
            f = Path(t) / "mine.png"
            f.write_bytes(png)
            page.set_input_files("#st-file", str(f))
            wait_js(page, "document.querySelector('#st-msg').textContent.startsWith('Picture 256')", timeout=20000)
        check(c_data(page).get("animated") == "", "/create/: your own picture loads")
        text = page.inner_text("body")
        hit = sorted({m.group(0) for m in HIDDEN_WORDS.finditer(text)})
        check(not hit, f"/create/: none of the words users never see {hit}")
        off = [u for u in asked if u.startswith("https://opendmd.dev/")]
        check(not off and any("/art-index/v1/SNES.json" in u for u in asked), f"/create/: Game art's index same-origin, nothing asked of opendmd.dev {off[:2]}")
        check(not page.evaluate("window.__csp || []"), f"/create/: no CSP violation {page.evaluate('window.__csp || []')[:2]}")
        page.context.close()


C_WHY = {  # create.js' words for the relay's refusals (site/worker/src/relay.ts `why`)
    "size": "That file is too big (over 16 MB): pick a shorter or smaller one",
    "type": "That address isn’t a picture or a video",
    "rate": "Too many tries in a minute: wait a moment, then try again",
    "busy": "opendmd.dev is fetching a lot for everyone just now: try again later, or download the picture and add the file here",
    "token": "Reload this page, then try again",
    "upstream": "That site refused to send it: download the picture, then add the file here",
    "redirects": "That site refused to send it: download the picture, then add the file here",
}


def create_links(browser, base: str, site: Path, errors: list[str]) -> None:
    """/create/'s addresses: `?url=` opens one; a host that allows the
    browser is asked directly, one that doesn't (no CORS: the fetch throws)
    through the relay, /api/fetch (stubbed here: tools/site-worker-test.py
    runs the real one); each refusal said in plain words."""
    png, gif = c_fixtures()
    page = new_page(browser, "laptop", errors)
    page.add_init_script("""document.addEventListener('securitypolicyviolation', e =>
        (window.__csp = window.__csp || []).push(e.violatedDirective + ' ' + e.blockedURI))""")
    page.route(lambda u: urllib.parse.urlsplit(u).path.startswith("/create/") and urllib.parse.urlsplit(u).path.endswith(("/", ".html")),
               lambda r: (lambda resp: r.fulfill(response=resp, headers={**resp.headers, "content-security-policy": csp_for(site, "/create/*")}))(r.fetch()))
    asked: list[str] = []
    answer = {"status": 200, "why": None}
    page.route("https://cors.example/**", lambda r: (asked.append(r.request.url), r.fulfill(status=200, body=gif, headers={
        "content-type": "image/gif", "access-control-allow-origin": "*"})))
    page.route("https://nocors.example/**", lambda r: (asked.append(r.request.url), r.abort()))

    minted: list[str] = []
    sent: list[str | None] = []

    def token(r):
        minted.append(r.request.method)
        r.fulfill(status=200, body=json.dumps({"token": f"t{len(minted)}", "ttl": 900}), headers={"content-type": "application/json"})
    page.route(f"{base}/api/fetch/token", token)

    def relay(r):
        asked.append(f"{r.request.method} {r.request.url} {r.request.post_data}")
        sent.append(r.request.headers.get("x-relay-token"))
        if answer["why"]:
            r.fulfill(status=answer["status"], body=json.dumps({"error": "x", "why": answer["why"]}), headers={"content-type": "application/json"})
        else:
            r.fulfill(status=200, body=gif, headers={"content-type": "image/gif", "x-relayed": "1"})
    page.route(f"{base}/api/fetch", relay)
    page.goto(f"{base}/create/?url=" + urllib.parse.quote("https://cors.example/wide.gif", safe=""))
    page.wait_for_selector("#create:not([aria-busy]) #studio .st-grid", timeout=30000)
    wait_js(page, "document.querySelector('#studio').dataset.animated === '1'", timeout=20000)
    check(asked == ["https://cors.example/wide.gif"] and page.inner_text("#st-msg").startswith("cors.example: 88 × 31 · 4 frames"),
          f"/create/?url=: opens the picture, asked directly when the site allows it ({asked}, '{page.inner_text('#st-msg')}')")
    asked.clear()
    page.click("#st-change")
    page.fill("#st-url", "https://nocors.example/pic.gif")
    page.click("#st-url-go")
    wait_js(page, "document.querySelector('#st-msg').textContent.startsWith('nocors.example: 88')", timeout=20000)
    check(asked == ["https://nocors.example/pic.gif", f"POST {base}/api/fetch https://nocors.example/pic.gif"],
          f"/create/: a site that refuses the browser, through the relay ({asked})")
    check(minted == ["POST"] and sent == ["t1"], f"/create/: the relay's token asked once (POST /api/fetch/token) and sent with the load ({minted} {sent})")
    for why, status in (("size", 413), ("type", 415), ("rate", 429), ("busy", 503), ("token", 403), ("upstream", 502), ("redirects", 502)):
        answer.update(status=status, why=why)
        if page.is_hidden("#st-url"):  # (Find stays open after a refusal)
            page.click("#st-change")
        page.fill("#st-url", f"https://nocors.example/{why}.gif")
        page.click("#st-url-go")
        wait_js(page, "!/^Loading/.test(document.querySelector('#st-msg').textContent)", timeout=10000)
        check(page.inner_text("#st-msg") == C_WHY[why], f"/create/: the relay's '{why}' said plainly ('{page.inner_text('#st-msg')}')")
        if why == "token":
            check(sent[-2:] == ["t1", "t2"] and len(minted) == 2, f"/create/: a refused token: a new one asked, the load tried once more, then said ({sent[-2:]} {minted})")
    check(not page.evaluate("window.__csp || []"), f"/create/ addresses: no CSP violation {page.evaluate('window.__csp || []')[:2]}")
    page.context.close()


VIDEO_FIXTURES = ROOT / "tools/fixtures/video"


def box_xs(im: Image.Image, frames: int) -> list[int]:
    """Where the yellow box starts in each frame of a GIF (-1: not seen)."""
    xs = []
    for i in range(frames):
        im.seek(i)
        rgb = im.convert("RGB")
        xs.append(min((x for x in range(128) for y in range(32) if (lambda p: p[0] > 180 and p[1] > 140 and p[2] < 110)(rgb.getpixel((x, y)))), default=-1))
    return xs


def create_video(browser, base: str, site: Path, errors: list[str], shots: Path | None) -> None:
    """Video in: a WebM (VP9) from a file and an MP4 (AV1: Playwright's
    Chromium has no H.264) through the relay (stubbed), each 2 s of 160x90
    at 30 frames a second with a yellow box moving right. Each becomes ~40
    frames (at most 20 a second) on the timeline, fitted to the panel;
    trimmed, downloaded: 128x32 with the box moving right."""
    page = new_page(browser, "laptop", errors)
    page.add_init_script("""document.addEventListener('securitypolicyviolation', e =>
        (window.__csp = window.__csp || []).push(e.violatedDirective + ' ' + e.blockedURI))""")
    page.route(lambda u: urllib.parse.urlsplit(u).path.startswith("/create/") and urllib.parse.urlsplit(u).path.endswith(("/", ".html")),
               lambda r: (lambda resp: r.fulfill(response=resp, headers={**resp.headers, "content-security-policy": csp_for(site, "/create/*")}))(r.fetch()))
    asked: list[str] = []
    page.on("request", lambda r: asked.append(urllib.parse.urlsplit(r.url).path))
    mp4 = (VIDEO_FIXTURES / "clip.mp4").read_bytes()
    page.route("https://nocors.example/**", lambda r: r.abort())
    page.route(f"{base}/api/fetch/token", lambda r: r.fulfill(status=200, body='{"token": "t", "ttl": 900}', headers={"content-type": "application/json"}))
    page.route(f"{base}/api/fetch", lambda r: r.fulfill(status=200, body=mp4, headers={"content-type": "video/mp4", "x-relayed": "1"}))
    page.goto(f"{base}/create/")
    page.wait_for_selector("#create:not([aria-busy]) #studio .st-grid", timeout=30000)
    check(not any(p.endswith("/create/video.js") for p in asked), "/create/ video: the video reader not loaded with the page")
    page.click("#st-src [data-v=own]")
    accept = page.get_attribute("#st-file", "accept")
    check(accept == "image/*,video/*" and "video" in page.inner_text("#st-own"), f"/create/ video: Choose takes any video ({accept})")
    page.set_input_files("#st-file", str(VIDEO_FIXTURES / "clip.webm"))
    wait_js(page, "document.querySelector('#studio').dataset.video === '1'", timeout=30000)
    d = c_data(page)
    n = int(d.get("srcFrames", 0))
    msg = page.inner_text("#st-msg")
    check(36 <= n <= 42 and msg == f"Video: 160 × 90 video · {n} frames" and d.get("animated") == "1" and d.get("fit") == "fit"
          and page.is_visible("#st-tl") and any(p.endswith("/create/video.js") for p in asked),
          f"/create/ video: a 2 s WebM from a file, {n} frames on the timeline, fitted to the panel ('{msg}', {d.get('fit')})")
    page.fill("#st-title", "Box")
    wait_js(page, "/^Box.*\\.gif$/.test(document.querySelector('#studio').dataset.name || '')", timeout=5000)
    if shots:
        page.evaluate("scrollTo(0, 0)")
        page.screenshot(path=str(shots / "create-video.png"), full_page=True)
    page.click("#st-tl .st-cell >> nth=0")
    page.click("#st-del")
    wait_js(page, f"document.querySelector('#studio').dataset.frames === '{n - 1}'", timeout=5000)
    im = c_download(page)
    xs = box_xs(im, im.n_frames)
    moving = [x for x in xs if x >= 0]
    check(im.size == (128, 32) and im.n_frames == n - 1 and len(moving) >= n // 2 and moving == sorted(moving) and moving[-1] > moving[0],
          f"/create/ video: downloaded 128x32, the first frame deleted, the box moving right ({im.n_frames} frames, box at {xs[:3]}…{xs[-3:]})")
    page.click("#st-change")
    page.fill("#st-url", "https://nocors.example/clip.mp4")
    page.click("#st-url-go")
    wait_js(page, "document.querySelector('#st-msg').textContent.startsWith('nocors.example: 160 × 90 video')", timeout=30000)
    d = c_data(page)
    check(36 <= int(d.get("srcFrames", 0)) <= 42 and d.get("video") == "1",
          f"/create/ video: an MP4 through the relay, {d.get('srcFrames')} frames ('{page.inner_text('#st-msg')}')")
    check(not page.evaluate("window.__csp || []"), f"/create/ video: no CSP violation {page.evaluate('window.__csp || []')[:2]}")
    page.context.close()



def create_elsewhere(browser, base: str, site: Path, errors: list[str], shots: Path) -> None:
    """/create/'s hand-offs: under Your own, other sites' search as links
    (new tab, "pixel art" and the words); the bookmarklet; the manifest and
    the service worker (scope /create/, the share target: a shared GIF as a
    multipart POST to /create/share opens in the Studio, a shared link as
    ?url=), and nothing else answered by the worker."""
    png, gif = c_fixtures()
    page = new_page(browser, "laptop", errors)
    page.add_init_script("""document.addEventListener('securitypolicyviolation', e =>
        (window.__csp = window.__csp || []).push(e.violatedDirective + ' ' + e.blockedURI))""")
    page.route(lambda u: urllib.parse.urlsplit(u).path.startswith("/create/") and urllib.parse.urlsplit(u).path.endswith(("/", ".html")),
               lambda r: (lambda resp: r.fulfill(response=resp, headers={**resp.headers, "content-security-policy": csp_for(site, "/create/*")}))(r.fetch()))
    page.route("https://cors.example/**", lambda r: r.fulfill(status=200, body=gif, headers={"content-type": "image/gif", "access-control-allow-origin": "*"}))
    page.goto(f"{base}/create/")
    page.wait_for_selector("#create:not([aria-busy]) #studio .st-grid", timeout=30000)
    page.click("#st-src [data-v=own]")
    names = page.eval_on_selector_all("#c-chips a", "els => els.map(e => e.textContent)")
    check(names == ["Tenor", "Klipy", "GIPHY", "Google", "Bing", "DuckDuckGo", "Reddit", "Tumblr", "DeviantArt", "Imgur"] and page.is_visible("#c-chips"),
          f"/create/ elsewhere: the search links under Your own ({names})")
    page.fill("#c-words", "mega man")
    links = dict(page.eval_on_selector_all("#c-chips a", "els => els.map(e => [e.textContent, e.href])"))
    attrs = page.eval_on_selector_all("#c-chips a", "els => els.map(e => e.target + ' ' + e.rel)")
    want = {"Tenor": "https://tenor.com/search/pixel-art-mega-man-gifs", "Klipy": "https://klipy.com/search/pixel-art-mega-man",
            "GIPHY": "https://giphy.com/search/pixel-art-mega-man",
            "Google": "https://www.google.com/search?tbm=isch&tbs=itp:animated&q=pixel+art+mega+man",
            "Bing": "https://www.bing.com/images/search?qft=+filterui:photo-animatedgif&q=pixel+art+mega+man",
            "DuckDuckGo": "https://duckduckgo.com/?iax=images&ia=images&iaf=type:gif&q=pixel+art+mega+man",
            "Reddit": "https://www.reddit.com/r/PixelArt/search/?restrict_sr=1&q=mega+man",
            "Tumblr": "https://www.tumblr.com/search/pixel%20art%20mega%20man",
            "DeviantArt": "https://www.deviantart.com/search?q=pixel+art+mega+man+gif", "Imgur": "https://imgur.com/search?q=pixel+art+mega+man"}
    bad = {k: v for k, v in links.items() if want.get(k) != v}
    check(not bad and set(attrs) == {"_blank noopener noreferrer"}, f"/create/ elsewhere: each a new tab with “pixel art” and the words {bad} {set(attrs)}")
    hint = page.inner_text("#c-elsewhere")
    check("Bring it back: paste its link or the picture here, or drag it onto this page." in hint, "/create/ elsewhere: the hint on bringing a GIF back")
    px = page.eval_on_selector("#c-pixilart", "a => [a.href, a.target, a.rel]")
    check(px == ["https://www.pixilart.com/draw", "_blank", "noopener noreferrer"] and "make the canvas 128 × 32" in hint,
          f"/create/ elsewhere: Pixilart to draw your own, in a new tab {px}")
    bm = page.get_attribute("#c-bm", "href") or ""
    check(bm == f"javascript:void(open('{base}/create/?url='+encodeURIComponent(location.href)))",
          f"/create/ elsewhere: the bookmarklet opens the page it is pressed on in Create Art ({bm[:90]})")
    page.click("#c-bm")  # (on our own page it does nothing: no navigation, no CSP report)
    if shots:
        page.evaluate("scrollTo(0, 0)")
        page.screenshot(path=str(shots / "create-elsewhere.png"), full_page=True)
    # The manifest and the worker.
    st, mf = page.evaluate("""async () => { const r = await fetch('manifest.webmanifest'); return [r.status, await r.json()]; }""")
    tgt = mf.get("share_target", {})
    check(st == 200 and mf.get("scope") == "/create/" and mf.get("start_url") == "/create/" and tgt.get("action") == "/create/share"
          and tgt.get("method") == "POST" and tgt.get("enctype") == "multipart/form-data" and tgt.get("params", {}).get("files", [{}])[0].get("name") == "media"
          and {i["sizes"] for i in mf.get("icons", [])} == {"192x192", "512x512"},
          f"/create/manifest.webmanifest: the share target (POST multipart, files as media), scope /create/, icons ({st})")
    icons = [page.evaluate(f"fetch('{i['src']}').then(r => r.ok && r.headers.get('content-type'))") for i in mf.get("icons", [])]
    check(icons == ["image/png", "image/png"], f"/create/: the icons are there ({icons})")
    scope = page.evaluate("navigator.serviceWorker.ready.then(r => r.scope)")
    check(scope == f"{base}/create/", f"/create/: its service worker, scope /create/ ({scope})")
    page.reload()
    page.wait_for_selector("#create:not([aria-busy]) #studio .st-grid", timeout=30000)
    check(page.evaluate("!!navigator.serviceWorker.controller"), "/create/: the page under its worker after a reload")
    # A shared GIF: the share sheet's multipart POST, as a form sends it.
    page.evaluate("""(b64) => { const bytes = Uint8Array.from(atob(b64), c => c.charCodeAt(0));
        const f = document.createElement('form'); f.method = 'POST'; f.action = 'share'; f.enctype = 'multipart/form-data';
        const i = document.createElement('input'); i.type = 'file'; i.name = 'media'; const t = new DataTransfer();
        t.items.add(new File([bytes], 'shared.gif', { type: 'image/gif' })); i.files = t.files; f.append(i);
        document.body.append(f); f.submit(); }""", __import__("base64").b64encode(gif).decode())
    page.wait_for_selector("#create:not([aria-busy]) #studio .st-grid", timeout=30000)
    wait_js(page, "document.querySelector('#studio').dataset.animated === '1'", timeout=20000)
    left = page.evaluate("caches.open('create-shared').then(c => c.keys()).then(k => k.length)")
    check(page.inner_text("#st-msg").startswith("Shared: 88 × 31 · 4 frames") and page.url == f"{base}/create/" and left == 0,
          f"/create/ share target: a shared GIF opens in the Studio, put aside only until then ('{page.inner_text('#st-msg')}', {page.url}, {left} left)")
    # A shared link (text with an address in it, as most apps share).
    page.evaluate("""() => { const f = document.createElement('form'); f.method = 'POST'; f.action = 'share'; f.enctype = 'multipart/form-data';
        for (const [k, v] of [['title', 'A GIF'], ['text', 'look at this https://cors.example/shared.gif']]) {
          const i = document.createElement('input'); i.name = k; i.value = v; f.append(i); }
        document.body.append(f); f.submit(); }""")
    page.wait_for_selector("#create:not([aria-busy]) #studio .st-grid", timeout=30000)
    wait_js(page, "document.querySelector('#st-msg').textContent.startsWith('cors.example: 88')", timeout=20000)
    check(urllib.parse.urlsplit(page.url).query == "url=https%3A%2F%2Fcors.example%2Fshared.gif",
          f"/create/ share target: a shared link opens as ?url= ({page.url})")
    # Nothing else is the worker's: a GET of the share address is the site's.
    st = page.evaluate("fetch('share').then(r => r.status)")
    check(st == 404, f"/create/: the worker answers only the share POST (GET /create/share: {st}, the site's 404)")
    check(not page.evaluate("window.__csp || []"), f"/create/ elsewhere: no CSP violation {page.evaluate('window.__csp || []')[:2]}")
    page.context.close()


PANEL_IMPORT_STUB = ("<!doctype html><title>waiting</title><script>addEventListener('message', (e) => { const d = e.data, g = d && d.gif; "
                     "document.title = JSON.stringify({origin: e.origin, data: {...d, gif: g instanceof ArrayBuffer ? "
                     "{bytes: g.byteLength, head: String.fromCharCode(...new Uint8Array(g, 0, 4))} : String(g)}}); });"
                     "if (opener) opener.postMessage({type: 'opendmd-import-ready'}, '*');</script>")


def create_send(browser, base: str, site: Path, errors: list[str]) -> None:
    """/create/'s Send to my panel: the address asked once (as /f/ asks it),
    the panel's page opened at `#import`, nothing in its link, and on its
    ready the GIF (a transferred ArrayBuffer) with its name, title,
    platform, author and logo, to that window's origin; Download still
    works after; remembered; a blocked pop-up said; no button before a picture."""
    png, gif = c_fixtures()
    page = new_page(browser, "laptop", errors)
    page.add_init_script("""document.addEventListener('securitypolicyviolation', e =>
        (window.__csp = window.__csp || []).push(e.violatedDirective + ' ' + e.blockedURI))""")
    page.route(lambda u: urllib.parse.urlsplit(u).path.startswith("/create/") and urllib.parse.urlsplit(u).path.endswith(("/", ".html")),
               lambda r: (lambda resp: r.fulfill(response=resp, headers={**resp.headers, "content-security-policy": csp_for(site, "/create/*")}))(r.fetch()))
    c_stub(page, base, png, gif, [])
    seen: list[str] = []
    page.context.route("http://192.168.1.50/**", lambda r: (seen.append(r.request.url), r.fulfill(body=PANEL_IMPORT_STUB, content_type="text/html")))
    page.goto(f"{base}/create/")
    page.wait_for_selector("#create:not([aria-busy]) #studio .st-grid", timeout=30000)
    check(page.is_hidden("#c-send"), "/create/ send: no Send to my panel until there is a picture")
    page.goto(f"{base}/create/?q=studio+test&platform=SNES")
    page.wait_for_selector("#find-grid .find-hit", timeout=30000)
    page.click("#find-grid .find-hit[data-kind=Named_Titles]")
    wait_js(page, "document.querySelector('#studio').dataset.name === 'SNES_Studio_Test_01.gif'", timeout=20000)
    page.check("#st-logo")
    page.fill("#st-author", "tester1")
    wait_js(page, "document.querySelector('#studio').dataset.name === 'SNES_Studio_Test_01_tester1_LOGO.gif'", timeout=10000)
    pri = page.eval_on_selector_all("#studio button.pri", "els => els.map(e => e.id)")
    check(pri == ["c-send"] and page.is_visible("#st-save"), f"/create/ send: Send to my panel the primary, Download beside it ({pri})")
    page.click("#c-send")
    check(page.is_visible("#c-ask") and page.input_value("#c-addr") == "opendmd.local" and not seen, "/create/ send: asks for the panel's address (opendmd.local)")
    page.fill("#c-addr", "example.com")
    page.click("#c-go")
    check("Press Send again" in page.inner_text("#c-addr-err") and not seen, "/create/ send: an address on the internet is asked about first")
    page.fill("#c-addr", "192.168.1.50")
    with page.context.expect_page() as info:
        page.click("#c-go")
    popup = info.value
    got = sent(popup)
    d = got["data"]
    want = {"type": "opendmd-import", "name": "SNES_Studio_Test_01_tester1_LOGO.gif", "title": "Studio Test", "platform": "SNES", "author": "tester1", "logo": True}
    check(popup.url == "http://192.168.1.50/#import" and got["origin"] == base and {k: d.get(k) for k in want} == want
          and d["gif"]["head"] == "GIF8" and d["gif"]["bytes"] > 100,
          f"/create/ send: the panel's page at #import, the GIF sent to it on its ready ({popup.url}, {str(got)[:120]})")
    popup.close()
    check(page.inner_text("#c-where-addr") == "192.168.1.50", "/create/ send: the address is remembered")
    im = c_download(page)
    check(im.size == (128, 32) and im.filename_suggested == "SNES_Studio_Test_01_tester1_LOGO.gif", "/create/ send: Download still works after sending")
    with page.context.expect_page() as info:
        page.click("#c-send")
    check(info.value.url == "http://192.168.1.50/#import" and sent(info.value)["data"]["gif"]["head"] == "GIF8", "/create/ send: straight to the panel the next time")
    info.value.close()
    page.evaluate("window.open = () => null")
    page.click("#c-send")
    check("allow pop-ups" in page.inner_text("#c-cap"), f"/create/ send: a blocked pop-up is said ({page.inner_text('#c-cap')[:60]})")
    check(not page.evaluate("window.__csp || []"), "/create/ send: no CSP violation")
    page.context.close()


def open_on_panel(browser, base: str, errors: list[str]) -> None:
    """Open on my panel, one press: every gallery tile has it (by name,
    `../f/#gallery=<slug>&panel=1`), leading through the Formula's page,
    which asks for the panel's address the first time and afterwards hands
    straight on (a top-level navigation, `http://<addr>/#gallery=<slug>`);
    the way back is the page without the mark."""
    every = examples()
    page = new_page(browser, "laptop", errors)
    seen: list[str] = []
    page.context.route("http://192.168.1.50/**", lambda r: (seen.append(r.request.url), r.fulfill(body="<p>panel</p>", content_type="text/html")))
    page.goto(f"{base}/formulas/")
    page.wait_for_selector(".ftile", timeout=20000)
    acts = page.eval_on_selector_all("#examples ~ ul .ftile", """els => els.map(e => [e.querySelector('a.fpic').getAttribute('href'),
        [...e.querySelectorAll('.facts a')].map(a => [a.textContent, a.getAttribute('href')])])""")
    good = len(acts) == len(every) and all(
        pic == f"../f/#gallery={e['slug']}" and links == [["Open on my panel", f"../f/#gallery={e['slug']}&panel=1"], ["Open", f"../f/#gallery={e['slug']}"]]
        for (pic, links), e in zip(acts, every))
    community = page.eval_on_selector_all("#community + ul .ftile", "els => els.map(e => e.querySelector('.facts a.add')?.getAttribute('href') || '')")
    check(good and all(re.fullmatch(r"\.\./f/#gallery=[a-z0-9-]+&panel=1", h) for h in community),
          f"gallery: Open on my panel and Open on every tile, by name ({len(acts)} examples, {len(community)} from the community)")
    i = next(k for k, e in enumerate(every) if e["bundle"])
    page.locator("#examples ~ ul .ftile").nth(i).locator("a.add").click()
    page.wait_for_selector("#f-ask:not([hidden])", timeout=20000)
    check(page.inner_text("#f-go") == "Open" and page.input_value("#f-addr") == "opendmd.local" and not seen,
          "gallery -> Open on my panel, no address yet: the Formula's page asks for it (opendmd.local)")
    page.fill("#f-addr", "192.168.1.50")
    with page.expect_navigation():
        page.click("#f-go")
    check(page.url == f"http://192.168.1.50/#gallery={every[i]['slug']}", f"gallery -> Open on my panel: to the panel by name ({page.url})")
    page.goto(f"{base}/formulas/")
    page.wait_for_selector(".ftile", timeout=20000)
    j = next(k for k, e in enumerate(every) if not e["bundle"])
    with page.expect_navigation(url=re.compile(r"^http://192\.168\.1\.50/"), timeout=20000):
        page.locator("#examples ~ ul .ftile").nth(j).locator("a.add").click()
    check(page.url == f"http://192.168.1.50/#gallery={every[j]['slug']}",
          f"gallery -> Open on my panel, the address known: one press to the panel ({page.url})")
    # Back: the gallery (the hand-off replaced the Formula's page, a
    # navigation before it had loaded) or that page without the mark; never
    # handed on again.
    n = len(seen)
    page.go_back()
    page.wait_for_load_state()
    page.wait_for_timeout(1000)
    check(page.url.startswith(base) and "panel=1" not in page.url and len(seen) == n,
          f"gallery -> Open on my panel: back is the site, not handed on again ({page.url[len(base):][:40]})")
    page.context.close()


def panel_end_to_end(browser, base: str, errors: list[str]) -> None:
    """Open on my panel, both ends: the site (this build, as https://opendmd.dev)
    sends an edited Formula to the panel's app (this build's web app on
    tools/mock-device.py over the demo card, as http://192.168.1.50); the
    app takes it from opendmd.dev and saves it under Your formulas, and it
    is still there, and on the card, after a reload."""
    card = ROOT / "target/demo-card"
    www = ROOT / "opendmd-web/www-dist"
    if not (card / "filelist.txt").is_file() or not (www / "pkg").is_dir():
        check(False, "panel end to end: needs make site (target/demo-card) and make web")
        return
    with tempfile.TemporaryDirectory(prefix="site-smoke-panel-") as tmp:
        overlay = Path(tmp)
        port = free_port()
        mock = subprocess.Popen(["uv", "run", "--quiet", str(ROOT / "tools/mock-device.py"), "--port", str(port), "--quiet",
                                 "--root", str(card), "--overlay", str(overlay), "--www", str(www)],
                                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        try:
            panel = f"http://127.0.0.1:{port}"
            for _ in range(200):
                try:
                    urllib.request.urlopen(panel + "/api/status", timeout=1).read()
                    break
                except OSError:
                    time.sleep(0.1)
            page = new_page(browser, "laptop", errors)

            def via(to: str, cors: bool):
                def handle(route):
                    u = urllib.parse.urlsplit(route.request.url)
                    # (The app's own requests, same-origin as on a panel: its origin is the mock's.)
                    h = {**route.request.headers, **({"origin": to} if "origin" in route.request.headers and not cors else {})}
                    r = route.fetch(url=to + u.path + (f"?{u.query}" if u.query else ""), headers=h)
                    route.fulfill(response=r, headers={**r.headers, **({"access-control-allow-origin": "*"} if cors else {})})
                return handle
            page.context.route("https://opendmd.dev/**", via(base, True))
            page.context.route("http://192.168.1.50/**", via(panel, False))
            slug = next(e["slug"] for e in examples() if not e["bundle"])
            page.goto("https://opendmd.dev/")
            page.evaluate("localStorage.setItem('odmd.display', '192.168.1.50')")
            page.goto(f"https://opendmd.dev/f/#gallery={slug}")
            f_ready(page)
            src = 'name "Site rings"\nloop 4\ngray(saw(x - t/4))'
            page.fill("#psi-src", src)
            wait_js(page, "document.querySelector('#f-name').textContent === 'Site rings'", timeout=5000)
            with page.context.expect_page() as info:
                page.click("#f-open")
            app = info.value
            with contextlib.suppress(PlaywrightTimeout):
                app.wait_for_selector("#toasts .toast", timeout=30000)
            t = " | ".join(app.eval_on_selector_all("#toasts .toast", "els => els.map(e => e.textContent)"))
            f = overlay / "formulas" / "site-rings.psi"
            check(f.is_file() and f.read_text() == src and "Your formulas" in t,
                  f"panel end to end: sent from the site, saved on the panel as it arrives ('{t}')")
            app.goto("http://192.168.1.50/#formulas")
            app.reload()
            app.wait_for_selector("#psi-ex .psi-tile", timeout=30000)
            mine = app.eval_on_selector_all("#psi-mine .psi-tile", "els => els.map(e => e.dataset.name)")
            check(mine == ["Site rings"] and f.is_file(), f"panel end to end: after a reload it is under Your formulas, and on the card ({mine})")
            page.context.close()
        finally:
            os.killpg(mock.pid, signal.SIGTERM)


def free_port() -> int:
    """A free local port (closed again for the mock to take)."""
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def demo_share(browser, base: str, errors: list[str]) -> None:
    """The demo's Formulas have no share link: Copy code, and in the
    editor Submit to the gallery."""
    page = new_page(browser, "laptop", errors)
    page.goto(f"{base}/demo/#formulas")
    page.wait_for_selector("#psi-ex .psi-tile", timeout=30000)
    page.click("#psi-ex .psi-tile >> nth=0")
    first = examples(bundled_only=True)[0]
    page.click("#psi-code-btn")
    page.wait_for_timeout(300)
    check(page.locator("#psi-share-btn, #psi-ed-share").count() == 0 and page.evaluate("navigator.clipboard.readText()") == first["src"],
          "demo: no share link; Copy code copies the Formula")
    page.click("#psi-edit")
    page.wait_for_selector("#psi-ed #psi-src", timeout=10000)
    page.click("#psi-ed-submit")
    page.wait_for_selector("#psi-submit-sheet #s-text", timeout=10000)
    check(page.inner_text("#s-text").startswith("-- author: "), "demo: Submit to the gallery opens its form")
    page.context.close()


def rows_ratio(page: Page) -> float:
    """The demo panel's even LED rows' brightness over its odd rows' (about
    1 for a picture; the Scanlines formula dims every other row)."""
    return page.evaluate("""() => { const c = document.querySelector('#now-panel'), r = c.width / 128;
        const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data, row = [0, 0];
        for (let y = 0; y < 32; y++) for (let x = 0; x < 128; x++) { const i = (((y * r + r / 2) | 0) * c.width + ((x * r + r / 2) | 0)) * 4; row[y % 2] += d[i] + d[i+1] + d[i+2]; }
        return row[1] ? row[0] / row[1] : 1; }""")


def demo_settings(browser, base: str, errors: list[str], shots: Path) -> None:
    """The demo's Settings run the device's code against its player in the
    page (js/demo.js): every tab and control there; brightness, colour, the
    Formula look, how long each GIF stays, the playlist and the schedule
    shown on the demo's panel (Now); panel timing tried and kept; what only
    a player can do (firmware, WiFi, restarting) shown with a line saying
    so; kept for the session (a reload), not beyond (a new one). Every tab
    at laptop and phone sizes in `shots`."""
    js = lambda e: page.evaluate(e)
    filt = lambda: js("getComputedStyle(document.querySelector('#now-panel')).filter")

    def tab(t):
        page.evaluate(f"location.hash = '#settings/{t}'")
        page.wait_for_selector(f"#st-tab-{t}", timeout=20000)
        page.wait_for_timeout(400)

    def now():
        page.evaluate("location.hash = '#now'")
        page.wait_for_selector("#view-now.on #now-panel")
        page.wait_for_timeout(700)

    def saved(sel):
        wait_js(page, f"document.querySelector({json.dumps(sel)}).closest('.card,.grp').querySelector('.tick')?.textContent.includes('Saved')", timeout=10000)

    page = new_page(browser, "laptop", errors)
    page.goto(f"{base}/demo/#now")
    wait_js(page, "(t => t && t !== 'Local simulator')(document.querySelector('#now-title')?.textContent)", timeout=30000)
    tab("look")
    check(all(page.locator(s).count() == 1 for s in ("#st-brightness", "#st-maxbright", "#st-look", "#st-glow")),
          "demo settings: Panel has the brightness, its maximum, the colour and the glow")
    page.evaluate("(e => { e.value = 30; e.dispatchEvent(new Event('input')); })(document.querySelector('#st-brightness'))")
    saved("#st-brightness")
    tab("play")
    check(page.locator("text=Device settings, WiFi and reboot live on the display").count() == 0
          and all(page.locator(s).count() == 1 for s in ("#st-set", "#st-plat-btn", "#st-dwell-min", "#st-dwell-max", "#st-shuffle")),
          "demo settings: Playback has the player's controls, not the local-mode sentence")
    page.fill("#st-dwell-min", "2")
    page.fill("#st-dwell-max", "5")
    page.press("#st-dwell-max", "Tab")
    saved("#st-dwell-min")
    sets = page.eval_on_selector_all("#st-set option", "os => os.map(o => o.value)")
    page.select_option("#st-set", "Demoscene")
    saved("#st-set")
    now()
    cap = page.inner_text("#view-now .capl .cap")
    check("brightness(0.5" in filt() and "2–5 s" in cap and page.input_value("#sim-list") == "/sets/Demoscene.txt",
          f"demo settings: brightness 30 dims the panel ({filt()}), each GIF 2–5 s ('{cap}'), the playlist Demoscene plays (of {sets})")
    tab("look")
    page.select_option("#st-look", "amber")
    saved("#st-look")
    page.select_option("#st-glow", "long")
    page.select_option("#st-trans", "wipe_left")
    saved("#st-trans")
    now()
    rgb = page.evaluate("""() => { const c = document.querySelector('#now-panel'), d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
        let r = 0, b = 0; for (let i = 0; i < d.length; i += 4) { r += d[i]; b += d[i+2]; } return [r, b]; }""")
    check(rgb[0] > 2 * rgb[1], f"demo settings: Amber tints the panel (red {rgb[0]:,}, blue {rgb[1]:,})")
    before = rows_ratio(page)
    tab("look")
    wait_js(page, "[...document.querySelectorAll('#st-look-formula option')].some(o => o.textContent === 'Scanlines')", timeout=20000)
    page.select_option("#st-look-formula", label="Scanlines")
    saved("#st-look-formula")
    now()
    page.wait_for_timeout(800)
    after = rows_ratio(page)
    check(abs(after / before - 1) > 0.2 and page.evaluate("import('./js/state.js').then(m => m.app.status.formula_look?.error ?? null)") is None,
          f"demo settings: the Scanlines look is drawn over the panel (even/odd rows {before:.2f} -> {after:.2f})")
    # A Formula's own `look` line on the demo's panel: Plasma (a gallery
    # Formula: the demo bundles the panel's ten) with `look screen 40` (as
    # the Formula settings sheet writes it), its path sent as the look; the
    # demo composes it as the player does (pkg-psi asLook).
    plasma = next(e["src"] for e in examples() if e["name"] == "Plasma")
    lum = lambda: page.evaluate("""() => { const c = document.querySelector('#now-panel'), d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
        let n = 0; for (let i = 0; i < d.length; i += 4) n += d[i] + d[i+1] + d[i+2]; return n; }""")
    l0 = lum()
    cfg = page.evaluate("""async (src) => { const { app } = await import('./js/state.js'), L = await import('./js/looks.js'), m = await L.psi();
        const f = L.set(m, { src }, 'screen', 40);
        await app.src.putConfig({ look_formula: L.path(m, f) }); const c = await app.refreshConfig();
        return [c.look_formula.startsWith('@formula?psb=') && !!m.asLookPsb(Uint8Array.from(atob(c.look_formula.slice(13).split('&')[0].replace(/-/g, '+').replace(/_/g, '/')), (ch) => ch.charCodeAt(0))) && !!(m.info()[5] & 32), 'look_blend' in c || 'look_formula_name' in c]; }""", plasma)
    now()
    page.wait_for_timeout(800)
    l1, ratio = lum(), rows_ratio(page)
    err = page.evaluate("import('./js/state.js').then(m => m.app.status.formula_look?.error ?? null)")
    check(cfg == [True, False] and err is None and l1 > 1.2 * l0 and abs(ratio - 1) < 0.15,
          f"demo settings: Plasma's `look screen 40` line drawn over the panel ({cfg}, {err}; brightness {l0:,} -> {l1:,}, even/odd rows {ratio:.2f})")
    page.screenshot(path=str(shots / "demo-look-plasma-laptop.png"))
    tab("look")
    page.click("#st-trans-preview")
    page.wait_for_timeout(1500)
    check(lit(page, "#st-trans-panel") > 500, "demo settings: the transition preview plays")
    # The schedule: night mode from a minute ago for an hour, Panel off.
    tab("time")
    wait_js(page, "/^\\d\\d:\\d\\d/.test(document.querySelector('#tm-now').textContent)", timeout=10000)
    hh, mm = map(int, page.inner_text("#tm-now")[:5].split(":"))
    t = hh * 60 + mm
    fmt = lambda m: f"{(m // 60) % 24:02d}:{m % 60:02d}"
    page.fill("#sc-night-from", fmt(t - 1 + 1440))
    page.fill("#sc-night-to", fmt(t + 60))
    page.select_option("#sc-night-mode", "off")
    page.check("#sc-night-on")
    page.wait_for_timeout(500)
    now()
    page.wait_for_selector("#now-banner:not([hidden])", timeout=10000)
    night = page.inner_text("#now-banner")
    check(night.startswith(f"Night mode until {fmt(t + 60)}") and filt() == "brightness(0)",
          f"demo settings: night mode turns the panel off, with Now's banner ('{night}', {filt()})")
    page.click("#now-wake")
    wait_js(page, "document.querySelector('#now-banner').textContent.startsWith('Schedule paused')", timeout=10000)
    check("brightness(0.5" in filt(), f"demo settings: Wake for an hour lights it again ({page.inner_text('#now-banner')}, {filt()})")
    page.click("#now-resume")
    wait_js(page, "document.querySelector('#now-banner').textContent.startsWith('Night mode')", timeout=10000)
    tab("time")
    page.uncheck("#sc-night-on")
    page.click("#sc-add")
    page.select_option("#sc-set", "Sprites")
    for d in page.query_selector_all("#sc-days .chip:not(.on)"):
        d.click()
    page.fill("#sc-from", "00:00")
    page.fill("#sc-to", "00:00")
    page.click("#sc-rule-save")
    page.wait_for_selector("#sc-rules [data-rule='0']", timeout=10000)
    now()
    rule = page.inner_text("#now-banner") if page.is_visible("#now-banner") else ""
    check(rule.startswith("Schedule: Sprites") and page.input_value("#sim-list") == "/sets/Sprites.txt",
          f"demo settings: a rule's playlist plays, with Now's banner ('{rule}')")
    tab("time")
    page.click("#sc-rules [data-rule='0'] button:has-text('Delete')")
    tab("replay")  # (Follow RePlayOS: a tab of its own)
    page.check("#fo-on")
    page.fill("#fo-host", "192.168.1.20")
    page.wait_for_timeout(700)
    page.click("#fo button:has-text('Test')")
    wait_js(page, "document.querySelector('#fo-result').textContent.includes('demo')", timeout=10000)
    check("can’t reach your network" in page.inner_text("#fo-result"), f"demo settings: Follow RePlayOS says the demo can't reach it ('{page.inner_text('#fo-result')}')")
    page.uncheck("#fo-on")
    # Device: status, card speed, panel timing tried and kept, the name; the rest said.
    tab("device")
    page.wait_for_selector("#st-fw .demo-cant", timeout=20000)
    lines = {g: page.inner_text(f"{g} .demo-cant") for g in ("#st-status", "#st-net", "#st-panel", "#st-fw")}
    check(all("demo" in v for v in lines.values()) and "Playlist" in page.inner_text("#st-facts"),
          f"demo settings: the status, and a line where the demo can't ({' | '.join(lines.values())})")
    for b in ("#st-reboot-btn", "#fw-go", "#fw-check", "#st-scan", "#pt-test"):
        page.click(b)
    toasts = page.eval_on_selector_all("#toasts .toast", "ts => ts.map(t => t.textContent)")
    check(sum("The demo can’t" in t or "The demo has no panel" in t for t in toasts) >= 5,
          f"demo settings: Reboot, Update, Check for updates, Scan and Test panel timing say so ({len(toasts)} toasts)")
    page.select_option("#st-card-speed", "fast")
    saved("#st-card-speed")
    page.select_option("#pt-latch_blanking", "2")
    page.wait_for_selector("#pt-keep:not([hidden])", timeout=10000)
    page.click("#pt-keep")
    page.wait_for_selector("#pt-bar[hidden]", state="attached", timeout=10000)
    kept = page.evaluate("import('./js/state.js').then(m => m.app.config.panel.latch_blanking)")
    check(kept == 2, f"demo settings: panel timing tried, then kept (latch blanking {kept})")
    page.fill("#st-hostname", "kitchen")
    page.click("#st-save-net")
    wait_js(page, "document.querySelector('#st-moved').textContent.includes('kitchen.local')", timeout=15000)
    page.fill("#st-ssid", "Elsewhere")
    page.click("#st-save-net")
    wait_js(page, "document.querySelector('#st-net .tick').textContent.includes('demo')", timeout=10000)
    check(True, f"demo settings: a new name is taken, a WiFi network refused ('{page.inner_text('#st-net .tick')}')")
    # Get beta updates: a device setting the demo keeps (its config knows
    # the field, so the save is not an "unknown field").
    page.check("#fw-beta")
    saved("#fw-beta")
    beta_on = page.evaluate("import('./js/state.js').then(m => m.app.config.beta_updates)")
    page.uncheck("#fw-beta")
    wait_js(page, "import('./js/state.js').then(m => m.app.config.beta_updates === false)", timeout=10000)
    check(beta_on is True and "Couldn" not in page.inner_text("#st-fw"), f"demo settings: Get beta updates saved on, then off ({beta_on})")
    page.check("#sec-key")
    page.wait_for_selector("#sec-key-val", timeout=10000)
    check(len(page.input_value("#sec-key-val")) == 24, "demo settings: Require a key shows one")
    page.uncheck("#sec-key")
    page.wait_for_timeout(300)
    check(page.evaluate("localStorage.getItem('odmd.apiKey')") is None, "demo settings: the key is not kept for the site's requests")
    # Kept for the session.
    page.reload()
    page.wait_for_selector("#st-tab-device", timeout=30000)
    tab("look")
    check(page.input_value("#st-look") == "amber" and page.input_value("#st-trans") == "wipe_left", "demo settings: kept across a reload")
    for t in ("play", "look", "time", "replay", "device"):
        tab(t)
        page.wait_for_timeout(600)
        page.screenshot(path=str(shots / f"demo-settings-{t}-laptop.png"), full_page=True)
    page.context.close()
    # A new session starts afresh; every tab on a phone.
    page = new_page(browser, "phone", errors)
    page.goto(f"{base}/demo/#settings/look")
    page.wait_for_selector("#st-tab-look", timeout=30000)
    check(page.input_value("#st-look") == "colour", "demo settings: a new session starts with a new player's settings")
    for t in ("play", "look", "time", "replay", "device"):
        tab(t)
        page.wait_for_timeout(600)
        check(page.evaluate("document.documentElement.scrollWidth <= innerWidth"), f"demo settings (phone): {t}, no sideways scroll")
        page.screenshot(path=str(shots / f"demo-settings-{t}-phone.png"), full_page=True)
    page.context.close()
    print(f"  shots in {shots}")


def demo_own_card(browser, base: str, site: Path, errors: list[str]) -> None:
    """Use your own card… in the demo: its player stops (no banner, no
    brightness filter, no settings of its own), and the folder plays in
    local mode with no errors."""
    page = new_page(browser, "laptop", errors)
    page.goto(f"{base}/demo/#now")
    wait_js(page, "(t => t && t !== 'Local simulator')(document.querySelector('#now-title')?.textContent)", timeout=30000)
    # Night mode now, so the demo's banner is up before the swap.
    page.evaluate("""() => import('./js/state.js').then(async ({ app }) => {
        await app.src.putSchedule({ rules: [], night: { from: '00:00', to: '00:00', mode: 'dim', brightness: 8 } }); })""")
    page.wait_for_selector("#now-banner:not([hidden])", timeout=10000)
    with tempfile.TemporaryDirectory() as d:
        card = Path(d) / "mycard"
        (card / "gifs/Kenney").mkdir(parents=True)
        gif = next((site / "demo/demo/gifs/Kenney").glob("*.gif"))
        shutil.copy(gif, card / "gifs/Kenney" / gif.name)
        (card / "filelist.txt").write_text(f"/gifs/Kenney/{gif.name}\n")
        page.set_input_files(".demo-own input", str(card))
        wait_js(page, "!document.querySelector('#where').textContent.startsWith('Demo')", timeout=10000)
        page.evaluate("location.hash = '#settings'")
        page.wait_for_selector("#view-settings.on .card", timeout=10000)
        page.evaluate("location.hash = '#now'")
        page.wait_for_selector("#view-now.on #sim-play", timeout=10000)
        page.select_option("#sim-list", "/filelist.txt")
        page.click("#sim-play")
        wait_js(page, f"document.querySelector('#now-title').textContent !== 'Local simulator'", timeout=10000)
        page.wait_for_timeout(1500)
        state = page.evaluate("""() => import('./js/state.js').then(({ app }) => ({
            banner: !!document.querySelector('#now-banner'), filter: getComputedStyle(document.querySelector('#now-panel')).filter,
            config: app.config, status: app.status, demo: !!app.demo, key: localStorage.getItem('odmd.apiKey') }))""")
        local = page.locator("#view-settings").inner_text()
    check(not state["banner"] and state["filter"] == "none" and state["config"] is None and state["status"] is None and not state["demo"]
          and "local mode" in local and lit(page, "#now-panel") > 50,
          f"demo: Use your own card… stops the demo's player (banner, filter, config: {state}) and plays the folder in local mode")
    page.context.close()


ART_PLATFORMS = ("MD", "SNES", "GB", "GBC", "GBA", "SATURN", "NDS", "NES", "SMS", "PCE", "PS1", "N64", "ARCADE")


def art_index(browser, base: str, site: Path, errors: list[str], cors_expected: bool = False) -> None:
    """The game name index at art-index/v1/, and the demo using it same-origin.
    `cors_expected`: served with the site's _headers (--base), so CORS `*` is checked."""
    bad = []
    cors = None
    for plat in ART_PLATFORMS:
        f = site / "art-index/v1" / f"{plat}.json"
        src = ROOT / "opendmd-web/art-index" / f"{plat}.json"
        if not f.is_file() or f.read_bytes() != src.read_bytes():
            bad.append(plat)
            continue
        req = urllib.request.Request(f"{base}/art-index/v1/{plat}.json", headers={"Origin": "http://192.168.1.50",
                                     # Browser Integrity Check (error 1010) refuses urllib's default user agent
                                     "User-Agent": "opendmd-site-test/1 (+https://opendmd.dev)"})
        with urllib.request.urlopen(req) as r:
            body, ctype, cors = r.read(), r.headers.get("content-type", ""), r.headers.get("access-control-allow-origin")
        d = json.loads(body)
        if r.status != 200 or "json" not in ctype or len(d["names"]) != len(d["kinds"]) or not d["names"]:
            bad.append(plat)
    check(not bad, f"art index: art-index/v1/<platform>.json for all {len(ART_PLATFORMS)} platforms, fetchable as JSON "
                   f"(CORS header here: {cors or 'none, the test server; the site sends *'}){' — bad: ' + ', '.join(bad) if bad else ''}")
    if cors_expected:
        check(cors == "*", f"art index: Access-Control-Allow-Origin: * from {base} ({cors})")
    check(not any(p.is_file() for p in (site / "demo/art").rglob("*")), "art index: not under demo/ (the www pack does not carry it)")
    png = io.BytesIO()
    Image.new("RGB", (32, 24), (200, 40, 40)).save(png, "PNG")
    page = new_page(browser, "laptop", errors)
    asked: list[str] = []
    site_asked: list[str] = []
    page.route("https://raw.githubusercontent.com/**", lambda r: r.fulfill(
        status=200, body=png.getvalue(), headers={"access-control-allow-origin": "*", "content-type": "image/png"}))
    # The site itself (util.js SITE), unless that is what is under test.
    if not base.startswith("https://opendmd.dev"):
        page.route("https://opendmd.dev/**", lambda r: (site_asked.append(r.request.url), r.abort()))
    page.on("request", lambda r: "/art-index/" in r.url and asked.append(r.url))
    page.goto(f"{base}/demo/#create")
    page.wait_for_selector("#view-create.on #studio .st-grid", timeout=30000)
    page.click("#st-src [data-v=gameart]")
    page.select_option("#find-platform", "GB")
    page.fill("#st-q", "tetris")
    page.click("#st-go")
    page.wait_for_selector("#find-grid .find-hit", timeout=30000)
    names = page.eval_on_selector_all("#find-grid .find-hit .find-t", "els => els.map(e => e.textContent)")
    check(asked == [f"{base}/art-index/v1/GB.json"] and not site_asked and names and all("tetris" in n.lower() for n in names),
          f"art index: the demo's Game art searches it same-origin ({asked}, {len(names)} tiles, e.g. {names[:1]}; "
          f"asked of opendmd.dev: {site_asked})")
    page.context.close()


def gallery(browser, base: str, errors: list[str], shots: Path | None) -> None:
    every = examples()
    for size in SIZES:
        page = new_page(browser, size, errors)
        page.goto(f"{base}/formulas/")
        page.wait_for_selector(".ftile", timeout=20000)
        wait_js(page, "[...document.querySelectorAll('.ftile img')].slice(0, 4).every(i => i.complete && i.naturalWidth == 128)",
                               timeout=20000)
        names = page.eval_on_selector_all("#examples ~ ul .ftile h3", "els => els.map(e => e.firstChild.textContent)")
        if size == "laptop":
            check(names == [e["name"] for e in every] and len(names) == 39, f"gallery: the {len(names)} examples, newest first")
            # What each says of itself: the bundled ten marked, the looks badged.
            marked = page.eval_on_selector_all("#examples ~ ul .ftile h3:has(.fbadge[title^='The web app'])", "els => els.map(e => e.firstChild.textContent)")
            looks = page.locator("#examples ~ ul .ftile h3 .fbadge", has_text="look").count()
            check(marked == [e["name"] for e in every if e["bundle"]] and looks >= 8,
                  f"gallery: the bundled ten marked ({marked}), {looks} looks badged")
            check(page.locator(".topnav a.on").inner_text() == "Gallery", "gallery: Gallery marked in the top bar")
            tour = page.get_attribute(".lead ~ p a[href$='guide/psi-1.html']", "href")
            check(tour == "../guide/psi-1.html", f"gallery: its intro links the guide's tour ({tour})")
            community = page.eval_on_selector_all("#community + ul .ftile h3", "els => els.map(e => e.textContent)")
            print(f"  ..   gallery: {len(community)} from the community {community[:5]}")
            page.evaluate("document.querySelectorAll('.ftile img').forEach(i => i.loading = 'eager')")
            wait_js(page, "[...document.querySelectorAll('.ftile img')].every(i => i.complete && i.naturalWidth == 128)", timeout=20000)
            dark = page.evaluate("""() => [...document.querySelectorAll('.ftile img')].filter(i => {
                const c = document.createElement('canvas'); c.width = 128; c.height = 32; const x = c.getContext('2d'); x.drawImage(i, 0, 0);
                const d = x.getImageData(0, 0, 128, 32).data; for (let k = 0; k < d.length; k += 4) if (d[k] + d[k+1] + d[k+2] > 90) return false; return true; }).length""")
            check(dark == 0, f"gallery: every thumbnail lit ({dark} dark)")
            page.locator("#examples ~ ul .ftile").nth(5).locator(".facts a.alt", has_text="Open").click()
            f_ready(page, 20000)
            e = every[5]
            about = re.search(r'^about "([^"]*)"', e["src"], re.M).group(1)
            check(page.inner_text("#f-name") == e["name"] and page.inner_text("#f-about") == about
                  and page.input_value("#psi-src").rstrip("\n") == e["src"].rstrip("\n"),
                  f"gallery: Open leads to {e['name']} in the editor, its name and about lines shown")
            # A Formula that draws over a GIF: shown over the sample picture, not black.
            e = next(x for x in every if x["name"] == "Scanlines")
            # From /f/ to /f/#…: only the hash changes, and f.js reloads on
            # hashchange, so the old page would answer the wait and vanish
            # under the next evaluate. Start from a blank page instead.
            page.goto("about:blank")
            page.goto(gallery_link(base, e["slug"]))
            wait_js(page, "+(document.querySelector('#psi-ed-panel')?.dataset.frames || 0) > 5", timeout=20000)
            n = lit(page, "#psi-ed-panel")
            check(n > 1000, f"/f/: {e['name']} (over a GIF) is lit, over the sample picture ({n} px)")
        else:
            check(page.evaluate("document.documentElement.scrollWidth <= innerWidth"), "gallery (phone): no sideways scroll")
        page.context.close()


# Sorting and filtering the gallery (ftsort.js): what each visible list shows.
SHOWN = """() => [...document.querySelectorAll('main ul.ftiles')].map(ul => [ul.closest('.fsec').dataset.part,
    [...ul.children].filter(li => !li.hidden && !li.closest('[hidden]')).map(li => li.querySelector('h3').firstChild.textContent)])"""


def gallery_sort(browser, base: str, errors: list[str]) -> None:
    """The gallery's sort and filter form: hidden without a script (the page
    newest first as built), shown with one; each sort's order and each
    filter's tiles as the entries say (formulas/gallery.json), the state in
    the address's query and back from it; by keyboard; nothing matching;
    hover-play after a sort and stopped when its tile is filtered out; no
    sideways scroll on a phone, inside the 16 px gutter."""
    every = examples()
    page = new_page(browser, "laptop", errors)
    page.goto(f"{base}/formulas/")
    page.wait_for_selector("#fsort:not([hidden])", timeout=20000)
    data = page.evaluate("fetch('gallery.json').then(r => r.json())")
    entries = [{**e, "part": "examples"} for e in data["examples"]] + [{**e, "part": "community"} for e in data["contributions"]]
    total = len(entries)

    def fold(s: str) -> str:
        return s.lower().replace("ß", "ss").replace("ς", "σ").replace("ſ", "s")

    def want(sort="new", show="all", controls=False, q=""):
        def ok(e):
            text = fold(f"{e['name']} {e['about']} by {e['author']}")
            return (all(w in text for w in fold(q).split())
                    and (show == "all" or (show == "community" and e["part"] == "community") or (show == "panel" and e["bundle"])
                         or (show == "looks" and e["look"]))
                    and (not controls or e["controls"]))
        out = []
        for part in ("community", "examples"):
            es = sorted((e for e in entries if e["part"] == part and ok(e)), key=lambda e: ([ord(c) for c in fold(e["name"])], e["slug"]))
            if sort != "name":
                dated = sorted((e for e in es if e.get("added")), key=lambda e: e["added"], reverse=sort == "new")
                es = dated + [e for e in es if not e.get("added")]
            if es:
                out.append([part, [e["name"] for e in es]])
        return out

    def shown():
        return [x for x in page.evaluate(SHOWN) if x[1]]

    def query():
        return dict(urllib.parse.parse_qsl(urllib.parse.urlsplit(page.url).query))

    count = page.inner_text("#fsort .fs-count")
    check(shown() == want() and query() == {} and count == f"{total} Formulas",
          f"gallery: the sort and filter form shown, newest first, no query ({count!r})")
    page.select_option("#fsort select[name=sort]", "old")
    check(shown() == want("old") and query() == {"sort": "old"}, f"gallery: Oldest ({page.url[len(base):]})")
    page.select_option("#fsort select[name=sort]", "name")
    check(shown() == want("name") and query() == {"sort": "name"}, "gallery: Name A–Z")
    for show in ("panel", "looks", "community"):
        page.select_option("#fsort select[name=show]", show)
        w = want("name", show)
        n = sum(len(x[1]) for x in w)
        check(shown() == w and query() == {"sort": "name", "show": show} and page.inner_text("#fsort .fs-count") == f"{n} of {total} Formulas"
              and (n > 0 or show == "community"), f"gallery: Show {show} ({n})")
    # An empty community list keeps its note when asked for.
    if not data["contributions"]:
        check(page.is_visible("main .fsec[data-part=community] .tiles-note") and not page.is_visible("main .fs-none"),
              "gallery: Show from the community with none yet: its note, not 'No Formulas match'")
    page.select_option("#fsort select[name=show]", "all")
    page.check("#fsort input[name=controls]")
    check(shown() == want("name", controls=True) and query() == {"sort": "name", "controls": "1"}, "gallery: Has controls")
    page.uncheck("#fsort input[name=controls]")
    page.fill("#fsort input[name=q]", "clock")
    w = want("name", q="clock")
    check(shown() == w and len(w[0][1]) >= 3 and query() == {"sort": "name", "q": "clock"}, f"gallery: search 'clock' ({w[0][1]})")
    # Linked: the same view after a reload, from the address alone.
    page.goto(f"{base}/formulas/?q=OpenDMD+ring&sort=old&show=all")
    page.wait_for_selector("#fsort:not([hidden])", timeout=20000)
    f = page.evaluate("[...new FormData(document.getElementById('fsort'))].map(([k, v]) => k + '=' + v).join('&')")
    check(shown() == want("old", q="OpenDMD ring") and f == "q=OpenDMD ring&sort=old&show=all",
          f"gallery: a linked view (?q=OpenDMD+ring&sort=old) opens as linked ({f})")
    # Nothing matching: said so, and Show all brings everything back.
    page.fill("#fsort input[name=q]", "no such formula zzz")
    check(page.is_visible("main .fs-none") and shown() == [] and not page.is_visible("main .fsec"),
          "gallery: nothing matching: 'No Formulas match', the lists hidden")
    page.click("main .fs-none button")
    check(shown() == want() and query() == {} and not page.is_visible("main .fs-none"), "gallery: Show all: everything, newest first again")
    # By keyboard: search, Sort, Show and Has controls in turn.
    page.focus("#fsort input[name=q]")
    page.keyboard.type("plasma")
    page.keyboard.press("Tab")
    on_sort = page.evaluate("document.activeElement.name")
    page.keyboard.press("ArrowDown")
    page.keyboard.press("Tab")
    page.keyboard.press("Tab")
    page.keyboard.press(" ")
    # (Space ticks a box on its keyup's click, which can land after press() returns.)
    with contextlib.suppress(Exception):
        wait_js(page, "location.search.includes('controls=1')", timeout=3000)
    st = query()
    check(on_sort == "sort" and st.get("q") == "plasma" and st.get("sort") == "old" and st.get("controls") == "1"
          and shown() == want("old", controls=True, q="plasma"), f"gallery: by keyboard (Tab, arrows, Space) {st}")
    # Hover-play after a sort, stopped when its tile is filtered out.
    page.goto(f"{base}/formulas/?sort=name")
    page.wait_for_selector("#fsort:not([hidden])", timeout=20000)
    tile = page.locator("li.ftile:has(a.fpic[href$='gallery=matrix'])")
    tile.locator("h3").hover()
    try:
        page.wait_for_selector("li.ftile[data-live=on] canvas", timeout=20000)
        live = tile.get_attribute("data-live") == "on"
    except Exception:
        live = False
    check(live, "gallery: a tile plays on hover after a sort")
    page.fill("#fsort input[name=q]", "starfield")
    page.wait_for_timeout(300)
    # (Another tile may come under the pointer and play: only this one's is asked about.)
    check(tile.locator("canvas").count() == 0 and tile.get_attribute("data-live") is None and not tile.is_visible(),
          "gallery: a playing tile filtered out stops playing")
    page.context.close()
    # A phone: no sideways scroll, the form inside the 16 px gutter.
    page = new_page(browser, "phone", errors)
    page.goto(f"{base}/formulas/?sort=name&show=looks&controls=1&q=a")
    page.wait_for_selector("#fsort:not([hidden])", timeout=20000)
    box = page.evaluate("(() => { const r = document.getElementById('fsort').getBoundingClientRect(); return [r.left, r.right]; })()")
    check(page.evaluate("document.documentElement.scrollWidth <= innerWidth") and box[0] >= 15.5 and box[1] <= 390 - 15.5,
          f"gallery (phone): the form inside the gutter, no sideways scroll ({box})")
    page.context.close()
    # No script: no form, the page as built (newest first).
    ctx = browser.new_context(viewport={"width": 1280, "height": 800}, java_script_enabled=False)
    page = ctx.new_page()
    page.goto(f"{base}/formulas/?sort=name")
    names = page.eval_on_selector_all("#examples ~ ul .ftile h3", "els => els.map(e => e.firstChild.textContent)")
    check(not page.is_visible("#fsort") and names == [e["name"] for e in every], "gallery (no script): no form, newest first as built")
    ctx.close()


# The site's structure (the design pass): the pages every visitor sees.
KEY_PAGES = {  # shot name: path
    "home": "/", "install": "/install/", "guide": "/guide/", "guide-web-app": "/guide/web-app.html",
    "formulas": "/formulas/", "create": "/create/", "studio": "/studio/", "dev": "/dev/", "dev-psi": "/dev/psi/", "dev-psi-errors": "/dev/psi/errors/", "licences": "/licenses/",
    "changelog": "/changelog/",
}
TOP_LABELS = ["Guide", "Gallery", "Create Art", "Demo", "For developers", "Install"]
# Words users never see (CLAUDE.md, "Product model"); "pack" only as the DMDoS GIF pack's name.
HIDDEN_WORDS = re.compile(r"\b(cache[sd]?|caching|ODM|odm|buckets?|manifests?|bundles?|bundled)\b|(?<!GIF )\bpack\b", re.I)


def page_links(site: Path, page: str) -> set[str]:
    """The published pages a built page links to (site paths, as files)."""
    text = (site / page).read_text(encoding="utf-8", errors="replace")
    out = set()
    for url in re.findall(r'href="([^"#?]*)', text):
        if not url or re.match(r"[a-z]+:", url):
            continue
        path = posixpath.normpath(url if url.startswith("/") else posixpath.join("/" + posixpath.dirname(page), url)).lstrip("/")
        if url.endswith("/") or (site / path).is_dir():
            path = (path + "/index.html").lstrip("/")
        if path.endswith(".html") and (site / path).is_file():
            out.add(path)
    return out


def reachable(site: Path) -> None:
    """Every page reachable from the home page by its links; the old
    addresses' redirect pages lead to pages that exist."""
    seen, todo = {"index.html"}, ["index.html"]
    while todo:
        for nxt in page_links(site, todo.pop()) - seen:
            seen.add(nxt)
            if not nxt.startswith("demo/"):
                todo.append(nxt)
    moved = {ln.split()[0].lstrip("/") for ln in (site / "_redirects").read_text().splitlines() if ln.strip()}
    pages = {p.relative_to(site).as_posix() for p in site.rglob("*.html")}
    # /stats is public but unlinked on purpose; /studio/ until the owner
    # says where its link goes.
    unlinked = {"404.html", "stats.html", "studio/index.html"}
    want = {p for p in pages if not p.startswith("demo/") and p not in unlinked and p not in moved}
    missing = sorted(want - seen)
    check(not missing and len(want) > 15, f"every page reachable from the home page by its links ({len(want)} pages){': ' + ', '.join(missing[:8]) if missing else ''}")
    bad = []
    for ln in (site / "_redirects").read_text().splitlines():
        old, new, code = ln.split()
        file = new.lstrip("/") + ("index.html" if new.endswith("/") else ".html")
        stub = site / old.lstrip("/")
        if code != "301" or not (site / file).is_file() or (old.endswith(".html") and not stub.is_file()):
            bad.append(ln)
    check(not bad, f"old addresses: every _redirects line a 301 to a page that exists, with its redirect page "
                   f"for a plain server{': ' + '; '.join(bad[:4]) if bad else ''}")
    for old in ("/gallery", "/guide/install", "/docs/index.html"):
        check(any(ln.split()[0] == old for ln in (site / "_redirects").read_text().splitlines()), f"old addresses: {old} is redirected")


def csp_pattern(path: str) -> str:
    for pat in ("/install/*", "/f/*", "/create/*", "/studio/*", "/dev/psi/*", "/formulas/*"):
        if path.startswith(pat[:-1]):
            return pat
    return "/*"


def structure(browser, base: str, site: Path, errors: list[str]) -> None:
    """The menu (the top bar on a laptop, the Menu button on a phone; the
    guide's and the developers' side menus, folded on a phone), the home
    page's way in, light and dark, no sideways scroll at phone width and no
    CSP violation (each page under its _headers policy) on the key pages,
    and the words users never see kept off their pages."""
    reachable(site)
    for size in SIZES:
        for scheme in ("light", "dark"):
            page = new_page(browser, size, errors, scheme=scheme)
            violations: list[str] = []
            page.add_init_script("""document.addEventListener('securitypolicyviolation', e =>
                (window.__csp = window.__csp || []).push(e.violatedDirective + ' ' + e.blockedURI))""")
            page.on("console", lambda m: "Content Security Policy" in m.text and violations.append(m.text[:160]))

            def with_csp(route):
                r = route.fetch()
                path = urllib.parse.urlsplit(route.request.url).path
                route.fulfill(response=r, headers={**r.headers, "content-security-policy": csp_for(site, csp_pattern(path))})
            page.route(lambda u: urllib.parse.urlsplit(u).path.endswith(("/", ".html")), with_csp)
            for name, path in KEY_PAGES.items():
                page.goto(base + path, wait_until="load")
                page.wait_for_timeout(150)
                what = f"{name} ({size}, {scheme})"
                wide = page.evaluate("document.documentElement.scrollWidth - innerWidth")
                check(wide <= 0, f"{what}: no sideways scroll ({wide} px)")
                seen = page.evaluate("window.__csp || []") + violations
                check(not seen, f"{what}: no CSP violation {seen[:2]}")
                violations.clear()
                bg = page.evaluate("getComputedStyle(document.body).backgroundColor")
                lum = sum(int(x) for x in re.findall(r"\d+", bg)[:3])
                check(lum > 600 if scheme == "light" else lum < 120, f"{what}: the {scheme} theme ({bg})")
                if scheme == "light" and size == "laptop" and name in ("home", "install", "guide", "formulas", "create", "studio", "changelog"):
                    text = page.inner_text("body")
                    hit = sorted({m.group(0) for m in HIDDEN_WORDS.finditer(text)})
                    check(not hit, f"{name}: none of the words users never see {hit}")
            # The menu.
            page.goto(base + "/guide/web-app.html")
            if size == "laptop":
                labels = page.eval_on_selector_all(".topnav a", "els => els.map(e => e.textContent)")
                check(labels == TOP_LABELS and page.is_visible(".topnav") and not page.is_visible(".mnav"),
                      f"menu ({scheme}): the top bar's {labels}")
                check(page.locator(".topnav a.on").inner_text() == "Guide" and page.locator(".side a.on").inner_text() == "The web app"
                      and not page.is_visible(".subnav"), "menu: the guide marked in the top bar, the page in the side menu")
                page.click(".topnav a:has-text('For developers')")
                page.wait_for_url("**/dev/")
                check(page.locator(".side a.on").inner_text() == "Building from source", "menu: For developers opens their side menu")
            else:
                check(not page.is_visible(".topnav") and page.is_visible(".mnav summary") and not page.is_visible(".side")
                      and page.is_visible(".subnav summary"), f"menu (phone, {scheme}): a Menu button; the guide's menu folded above the page")
                check("The web app" in page.inner_text(".subnav summary"), "menu (phone): the folded menu names the page")
                page.click(".mnav summary")
                labels = [t.strip() for t in page.eval_on_selector_all(".mnav nav a", "els => els.map(e => e.innerText)")]
                check(labels == TOP_LABELS and page.is_visible(".mnav nav a:has-text('Gallery')"), f"menu (phone): Menu opens the links {labels}")
                page.click(".mnav nav a:has-text('Gallery')")
                page.wait_for_url("**/formulas/")
                check(page.locator(".ftile").count() > 30, "menu (phone): Gallery opens the gallery")
            # The home page's way in.
            if scheme == "light":
                page.goto(base + "/")
                hrefs = page.eval_on_selector_all(".hero .cta", "els => els.map(e => e.getAttribute('href'))")
                doors = page.eval_on_selector_all("[aria-labelledby=start] .door", "els => els.map(e => e.getAttribute('href'))")
                more = page.eval_on_selector_all("[aria-labelledby=more] .door", "els => els.map(e => e.getAttribute('href'))")
                check(hrefs == ["install/", "demo/"] and doors == ["install/", "guide/"] and more == ["formulas/", "create/", "dev/"],
                      f"home ({size}): Install and the demo up top {hrefs}; Install and the guide to get started {doors}; "
                      f"the gallery, Create Art and For developers under Explore more {more}")
                art = page.eval_on_selector_all(".hero img, .door img", "els => els.filter(i => i.getAttribute('src').startsWith('art/')).length")
                check(art == 3 and page.inner_text("h1").strip() != "" and not page.query_selector(".tiles"),
                      f"home ({size}): a heading, the panel and the two doors' pictures ({art} pieces of art), no grid of tiles")
                # The hero's three measured numbers under its buttons, its scene behind it
                # (the wide one, or the phone's), and the one frame round every still
                # panel and screenshot; headings in the display face, once it has loaded.
                proof = page.eval_on_selector_all(".hero .ctas ~ .proof li b", "els => els.map(e => e.textContent.trim())")
                check(len(proof) == 2 and all(re.search(r"\d", n) for n in proof), f"home ({size}): two numbers under the hero's buttons {proof}")
                scene = page.evaluate("getComputedStyle(document.querySelector('.hero')).backgroundImage")
                want_scene = "den-phone.webp" if size == "phone" else "den.webp"
                loaded = page.evaluate("fetch(%r).then(r => r.ok && r.headers.get('content-type'))" % want_scene)
                check(want_scene in scene and loaded == "image/webp", f"home ({size}): the hero's scene is {want_scene} ({loaded})")
                framed = page.evaluate("[document.querySelectorAll('section.focus figure > .frame > img').length, document.querySelectorAll('.door > .frame > .led').length]")
                check(framed == [5, 2], f"home ({size}): every screenshot and still panel in the frame {framed}")
                wait_js(page, "document.fonts.check('600 40px Fraunces') && document.fonts.check('16px Silkscreen')")
                faces = page.evaluate("['.hero h1', 'section.focus h2', '.hero .chip'].map(s => getComputedStyle(document.querySelector(s)).fontFamily.split(',')[0])")
                check(faces == ["Fraunces", "Fraunces", "Silkscreen"], f"home ({size}): headings in Fraunces, the label in Silkscreen {faces}")
                # The five focus bands, in the owner's order, each words beside a
                # screenshot of the web app that has loaded (they load lazily: each
                # scrolled to): side by side on a laptop, every other one swapped,
                # the words first on a phone.
                page.eval_on_selector_all("section.focus figure", "els => els.forEach(f => f.scrollIntoView())")
                wait_js(page, "[...document.querySelectorAll('section.focus img')].every(i => i.complete && i.naturalWidth > 0)")
                page.evaluate("scrollTo(0, 0)")
                bands = page.evaluate("""[...document.querySelectorAll('section.focus')].map(s => {
                    const t = s.querySelector(':scope > div').getBoundingClientRect(), f = s.querySelector(':scope > figure').getBoundingClientRect();
                    return [s.querySelector('h2').textContent, s.querySelector('img').getAttribute('src'),
                            f.left >= t.right - 1 ? 'right' : f.right <= t.left + 1 ? 'left' : f.top >= t.bottom - 1 ? 'below' : 'over'];
                })""")
                want = [["Nothing to install on your phone", "docs/images/phone-now.png"], ["Follows RePlayOS", "docs/images/now-follow.png"],
                        ["Playlists", "docs/images/library.png"], ["Brightness and colour", "docs/images/settings-look.png"],
                        ["Night mode and schedules", "docs/images/settings-schedule.png"]]
                check([b[:2] for b in bands] == want, f"home ({size}): the five bands in order, each with its screenshot {[b[:2] for b in bands]}")
                sides = [b[2] for b in bands]
                check(sides == (["below"] * 5 if size == "phone" else ["right", "left", "right", "left", "right"]),
                      f"home ({size}): the screenshots {'below their words' if size == 'phone' else 'beside their words, the sides alternating'} {sides}")
                # Formulas are named only under Explore more (the menus aside).
                named = page.evaluate("""[...document.querySelectorAll('main section:not([aria-labelledby=more])')]
                    .filter(s => /formula/i.test(s.innerText)).map(s => s.getAttribute('aria-labelledby') || s.className)""")
                check(not named, f"home ({size}): Formulas named only under Explore more {named}")
                # The hero's Formula plays live once the page has loaded (home.js).
                wait_js(page, "+document.querySelector('.hero-panel [data-live=on] canvas')?.dataset.frames > 10")
                check(True, f"home ({size}): the hero plays its Formula live")
                if size == "phone":
                    hero = page.evaluate("document.querySelector('.hero-panel').getBoundingClientRect().top < document.querySelector('.hero h1').getBoundingClientRect().top")
                    check(hero, "home (phone): the panel comes first")
            page.context.close()


SCENES = ("synth", "night", "day", "chunky", "shelf", "neon", "arcade")  # tools/site.py SCENES: the first is everyone's


def luminance(rgb) -> float:
    """WCAG relative luminance of an (r, g, b) of 0..255."""
    lin = [(c / 255 / 12.92) if c / 255 <= 0.03928 else ((c / 255 + 0.055) / 1.055) ** 2.4 for c in rgb[:3]]
    return 0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2]


def ink_contrast(page: Page, selector: str) -> float:
    """The contrast of an element's ink against what is really behind it: the
    element is photographed with its words hidden, and the ink is set against
    the worst of that picture (its lightest twentieth for light ink, its
    darkest for dark)."""
    ink = luminance([int(float(x)) for x in re.findall(r"[\d.]+", page.evaluate(f"getComputedStyle(document.querySelector({selector!r})).color"))[:3]])
    box = page.evaluate(f"(e => {{ e.style.visibility = 'hidden'; const r = e.getBoundingClientRect(); return [r.left + scrollX, r.top + scrollY, r.width, r.height]; }})(document.querySelector({selector!r}))")
    shot = page.screenshot(clip={"x": box[0], "y": box[1], "width": box[2], "height": box[3]}, full_page=True)
    page.evaluate(f"document.querySelector({selector!r}).style.visibility = ''")
    im = Image.open(io.BytesIO(shot)).convert("RGB")
    im.thumbnail((240, 240))
    lums = sorted(luminance(px) for px in im.getdata())
    behind = lums[int(len(lums) * 0.95)] if ink > 0.3 else lums[int(len(lums) * 0.05)]
    hi, lo = max(ink, behind), min(ink, behind)
    return (hi + 0.05) / (lo + 0.05)


def scenes(browser, base: str, site: Path, errors: list[str]) -> None:
    """The home page's scenes (site/theme/SCENES.md): /?scene=NAME shows each
    of them, with its own picture and no other's fetched, its words legible
    over it in both themes and nothing wider than a phone; the name is
    remembered, `synth` forgets it, a name that isn't one changes nothing,
    and with no storage the address still works."""
    css, js = (site / "site.css").read_text(), (site / "scene.js").read_text()
    listed = re.search(r"all = \[([^\]]*)\]", js)
    check(listed is not None and tuple(re.findall(r'"(\w+)"', listed.group(1))) == SCENES
          and all(f"[data-scene={n}]" in css and (site / f"den-{n}.webp").is_file() and (site / f"den-{n}-phone.webp").is_file() for n in SCENES[1:]),
          f"scenes: scene.js, site.css and the pictures agree on {', '.join(SCENES)}")

    def open_at(page: Page, address: str) -> list[str]:
        """Go there; the scene pictures the page fetched, by name (and anything that went wrong with one)."""
        got: list[str] = []
        seen = lambda r: re.search(r"/den[^/]*\.webp$", r.url) and got.append(r.url.rsplit("/", 1)[1] + ("" if r.status == 304 or r.ok and "image/webp" in r.headers.get("content-type", "") else f" ({r.status})"))
        page.on("response", seen)
        page.goto(base + address, wait_until="load")
        wait_js(page, "document.fonts.status === 'loaded' && getComputedStyle(document.querySelector('.hero')).backgroundImage !== 'none'")
        page.wait_for_timeout(300)
        page.remove_listener("response", seen)
        return got

    named = "document.documentElement.getAttribute('data-scene')"
    for scene in SCENES:
        for size in SIZES:
            want = "den" + ("" if scene == SCENES[0] else f"-{scene}") + ("-phone" if size == "phone" else "") + ".webp"
            for scheme in ("light", "dark"):
                page = new_page(browser, size, errors, scheme)
                got = open_at(page, f"/?scene={scene}")
                bg = page.evaluate("getComputedStyle(document.querySelector('.hero')).backgroundImage")
                check(got == [want] and want in bg and page.evaluate(named) == (None if scene == SCENES[0] else scene),
                      f"scene {scene} ({size}, {scheme}): the hero has {want} behind it, the only scene fetched {got}")
                low = {sel: round(c, 1) for sel in (".hero h1", ".hero .lead", ".hero .proof", ".hero .ticks", ".hero-panel .cap")
                       if (c := ink_contrast(page, sel)) < (3 if sel == ".hero h1" else 4.5)}
                check(not low, f"scene {scene} ({size}, {scheme}): the hero's words read over it (contrast under 4.5, a heading's under 3: {low or 'none'})")
                if size == "phone":
                    wide = page.evaluate("[document.documentElement.scrollWidth, innerWidth]")
                    check(wide[0] <= wide[1], f"scene {scene} (phone, {scheme}): nothing wider than the screen {wide}")
                page.context.close()
    # The switch: asked for, remembered, an unknown name, back to everyone's, no storage.
    page = new_page(browser, "laptop", errors)
    first = open_at(page, "/")
    check(first == ["den.webp"] and page.evaluate(named) is None, f"scenes: a first visit gets everyone's scene {first}")
    asked, kept = open_at(page, "/?scene=shelf"), open_at(page, "/")
    check(asked == kept == ["den-shelf.webp"] and page.evaluate(named) == "shelf", f"scenes: ?scene=shelf shows it, and the next visit remembers {asked} {kept}")
    odd = open_at(page, "/?scene=no-such-scene")
    odd_name, after = page.evaluate(named), open_at(page, "/")
    check(odd == ["den.webp"] and odd_name is None and after == ["den-shelf.webp"],
          f"scenes: a name that isn't one shows everyone's scene {odd} and leaves the remembered one alone {after}")
    back, then = open_at(page, f"/?scene={SCENES[0]}"), open_at(page, "/")
    check(back == then == ["den.webp"] and page.evaluate(named) is None, f"scenes: ?scene={SCENES[0]} goes back to everyone's and forgets {back} {then}")
    page.evaluate("localStorage.setItem('opendmd-scene', 'gone-since')")
    stale = open_at(page, "/")
    check(stale == ["den.webp"] and page.evaluate(named) is None, f"scenes: a remembered name that is no longer a scene falls back to everyone's {stale}")
    page.context.close()
    page = new_page(browser, "laptop", errors)
    page.add_init_script("Object.defineProperty(window, 'localStorage', { get() { throw new DOMException('no storage', 'SecurityError'); } })")
    bare, plain = open_at(page, "/?scene=neon"), None
    name = page.evaluate(named)
    plain = open_at(page, "/")
    check(bare == ["den-neon.webp"] and name == "neon" and plain == ["den.webp"], f"scenes: with no storage the address still works {bare}, nothing is remembered {plain}")
    page.context.close()


def hero_show(browser, base: str, errors: list[str]) -> None:
    """The home page's hero plays its little show (hero.js): it goes live,
    moves on to the next item with its caption and no layout shift and no
    console error; it stops off-screen and on a click; with less motion
    asked for, the still stays."""
    hero = "document.querySelector('.hero-panel .bezel')"
    frames = f"+{hero}.querySelector('canvas')?.dataset.frames || 0"
    for size in SIZES:
        page = new_page(browser, size, errors)
        console: list[str] = []
        page.on("console", lambda m: m.type in ("error", "warning") and console.append(m.text[:160]))
        page.on("pageerror", lambda e: console.append(str(e)[:160]))
        page.goto(base + "/")
        wait_js(page, f"{hero}.dataset.live === 'on' && {hero}.dataset.item === '0' && ({frames}) > 5", timeout=20000)
        items = page.eval_on_selector_all("#hero-show li", "els => els.map(e => e.textContent.trim())")
        box = "(r => [r.top, r.height])(document.querySelector('.hero-panel').getBoundingClientRect())"
        h1 = "document.querySelector('.hero h1').getBoundingClientRect().top"
        before = page.evaluate(f"[{box}, {h1}]")
        check(len(items) >= 4 and page.inner_text(".hero-panel .cap").strip() == items[0],
              f"home ({size}): the hero's show is live on its first item ({len(items)} items)")
        wait_js(page, f"{hero}.dataset.item === '1'", timeout=20000)
        after = page.evaluate(f"[{box}, {h1}]")
        check(page.inner_text(".hero-panel .cap").strip() == items[1],
              f"home ({size}): the show moves on to its second item, the caption with it")
        check(before == after, f"home ({size}): no layout shift as the show moves on {before} {after}")
        # Off-screen it stops; back on screen it goes on.
        page.evaluate("scrollTo(0, document.body.scrollHeight)")
        page.wait_for_timeout(300)
        n = page.evaluate(frames)
        page.wait_for_timeout(500)
        check(page.evaluate(frames) == n, f"home ({size}): the show stops off-screen")
        page.evaluate("scrollTo(0, 0)")
        wait_js(page, f"({frames}) > {n}", timeout=5000)
        # A click pauses it, another plays it.
        page.click(".hero-panel .bezel")
        page.wait_for_timeout(200)
        n = page.evaluate(frames)
        page.wait_for_timeout(500)
        check(page.evaluate(f"{hero}.hasAttribute('data-paused')") and page.evaluate(frames) == n,
              f"home ({size}): a click pauses the show")
        page.click(".hero-panel .bezel")
        wait_js(page, f"({frames}) > {n}", timeout=5000)
        check(not console, f"home ({size}): no console errors from the show {console[:2]}")
        page.context.close()
    # Less motion: the still stays, nothing plays.
    page = new_page(browser, "laptop", errors)
    page.emulate_media(reduced_motion="reduce")
    page.goto(base + "/")
    page.wait_for_timeout(2500)
    check(page.evaluate(f"!{hero}.querySelector('canvas') && !!{hero}.querySelector('picture.led img')"),
          "home: with less motion asked for, the hero keeps its still")
    page.context.close()


def screenshots(browser, base: str, errors: list[str], shots: Path) -> None:
    """The key pages, /f/ (an example, as the gallery's Open gives it) and
    the gallery, at both sizes in both themes: the whole page
    (<name>-<size>-<theme>.png), plus the first screen of each at laptop and
    phone sizes (-top) and the phone's open Menu."""
    e = next(x for x in examples() if x["name"] == "Stripes")  # a slider
    pages = {**KEY_PAGES, "f": gallery_link(base, e["slug"])[len(base):], "not-found": "/no/such/page"}
    for size in SIZES:
        for scheme in ("light", "dark"):
            page = new_page(browser, size, [], scheme=scheme)  # (the 404 page answers 404)
            for name, path in pages.items():
                page.goto("about:blank")
                page.goto(base + path)
                if name == "f":
                    wait_js(page, "+(document.querySelector('#psi-ed-panel')?.dataset.frames || 0) > 20", timeout=20000)
                page.evaluate("document.querySelectorAll('img[loading=lazy]').forEach(i => i.loading = 'eager')")
                wait_js(page, "[...document.images].every(i => i.complete)", timeout=20000)
                page.wait_for_timeout(600)
                page.screenshot(path=str(shots / f"{name}-{size}-{scheme}-top.png"))
                page.screenshot(path=str(shots / f"{name}-{size}-{scheme}.png"), full_page=True)
            if size == "phone":
                page.goto(base + "/guide/web-app.html")
                page.click(".mnav summary")
                page.wait_for_timeout(200)
                page.screenshot(path=str(shots / f"menu-{size}-{scheme}.png"))
            page.context.close()
    print(f"  shots in {shots}")


def api_pages(browser, site: Path, dist: Path, beta_dist: Path | None, art: Path, errors: list[str]) -> None:
    page_html = site / "dev/api.html"
    check(page_html.is_file() and "HTTP API" in page_html.read_text(), "api: dev/api.html is in the site")
    carried = sorted(p.name for p in (site / "dev/api").glob("v*.html"))
    text = page_html.read_text() if page_html.is_file() else ""
    check(all(f'href="api/{n}"' in text for n in carried), f"api: dev/api.html links every frozen copy the site carries {carried}")
    # The frozen page is this tree's release: the beta, when one is newer than the release.
    ver = json.loads(((beta_dist or dist) / "manifest-4mb.json").read_text())["version"]
    with tempfile.TemporaryDirectory() as t:
        tmp = Path(t)
        arch = tmp / "archive"
        arch.mkdir()
        old = arch / "v0.0.1.html"
        old.write_text('<!doctype html><meta charset="utf-8"><title>HTTP API (0.0.1)</title><p>OpenDMD 0.0.1</p>\n')
        (arch / "notes.html").write_text("<p>not a version</p>\n")
        (arch / "v0.0.2.html").symlink_to(old)
        out = tmp / "site"
        for d in ("dev", "fonts", "art"):
            shutil.copytree(site / d, out / d)
        shutil.copyfile(site / "site.css", out / "site.css")

        def build(freeze: str) -> str:
            r = subprocess.run(["uv", "run", "--quiet", str(ROOT / "tools/site.py"), str(out), "--api-only", "--dist", str(dist),
                                *(["--beta-dist", str(beta_dist)] if beta_dist else []),
                                "--art", str(art), "--api-archive", str(arch), "--api-freeze", freeze],
                               cwd=ROOT, capture_output=True, text=True)
            check(r.returncode == 0, f"api: site.py --api-only --api-freeze {freeze} ({r.stderr.strip()[-200:]})")
            return r.stdout
        build("yes")
        names = sorted(p.name for p in (out / "dev/api").iterdir())
        check(names == sorted([f"v{ver}.html", "v0.0.1.html"]), f"api: the frozen pages are this release's and the archive's, nothing else {names}")
        check(filecmp.cmp(old, out / "dev/api/v0.0.1.html", shallow=False), "api: an older release's frozen page is kept byte for byte")
        latest = (out / "dev/api.html").read_text()
        m = re.search(r'<p class="vers">(.*?)</p>', latest)
        listed = re.findall(r'href="api/v([^"]+)\.html"', m.group(1)) if m else []
        check(listed == [ver, "0.0.1"], f"api: dev/api.html lists the frozen pages, newest first {listed}")
        frozen = (out / f"dev/api/v{ver}.html").read_text()
        check(f"describes the HTTP API of <strong>OpenDMD {ver}</strong>" in frozen and 'href="../api.html"' in frozen
              and f"<title>OpenDMD HTTP API ({ver}) · OpenDMD</title>" in frozen,
              "api: the frozen page says which release it describes and links the latest")
        base = serve(out)
        page = new_page(browser, "laptop", errors)
        page.goto(base + "/dev/api.html")
        page.click(f'p.vers a[href="api/v{ver}.html"]')
        page.wait_for_url(f"**/dev/api/v{ver}.html")
        check(f"OpenDMD {ver}" in page.inner_text("p.vers") and page.locator("main h2").count() > 10,
              f"api: dev/api.html's link opens the frozen page ({page.locator('main h2').count()} sections)")
        page.click("p.vers a")
        page.wait_for_url("**/dev/api.html")
        check(page.locator('.side a.on[href="api.html"]').count() == 1, "api: the frozen page's link leads back to the latest")
        page.context.close()
        # Off a tag (`git describe` is not v<version>), auto freezes nothing.
        desc = subprocess.run(["git", "describe", "--always", "--dirty", "--tags", "--match", "v*"], cwd=ROOT, capture_output=True, text=True).stdout.strip()
        if desc != f"v{ver}":
            build("auto")
            names = sorted(p.name for p in (out / "dev/api").iterdir())
            check(names == ["v0.0.1.html"], f"api: off the tag ({desc}), --api-freeze auto freezes nothing new {names}")


def csp_for(site: Path, pattern: str) -> str:
    """The Content-Security-Policy _headers sets for `pattern`'s rule (one
    CSP per path: a rule of its own replaces the docs' one)."""
    for rule in (site / "_headers").read_text().split("\n\n"):
        lines = rule.splitlines()
        if lines and lines[0] == pattern:
            return next(l.split(": ", 1)[1] for l in lines if l.strip().startswith("Content-Security-Policy:"))
    raise SystemExit(f"_headers: no rule for {pattern}")


def on_this_page(site: Path) -> None:
    """The long pages open with a folded "On this page" list of their
    sections, each link to a heading on the page."""
    for page in ("dev/api.html", "guide/web-app.html", "dev/psi/index.html", "dev/psi/errors/index.html"):
        text = (site / page).read_text(encoding="utf-8")
        m = re.search(r'<details class="onpage"><summary>On this page</summary><ul>(.*?)</ul></details>', text)
        ids = re.findall(r'href="#([^"]+)"', m.group(1)) if m else []
        missing = [i for i in ids if f'id="{i}"' not in text]
        check(len(ids) >= 3 and not missing and text.index("</h1>") < m.start() < text.index("<h2"),
              f"{page}: \"On this page\" under the heading, {len(ids)} sections{', missing ' + str(missing[:3]) if missing else ''}")


def psi_reference(browser, base: str, site: Path, errors: list[str], shots: Path) -> None:
    """/dev/psi/, the Psi language reference, and /dev/psi/errors/, its
    error codes: in the developers' menu (the reference linked from every
    page's footer); the reference's generated tables (each row an id, the
    "Every name" index linking them), the error page's index, each code an
    anchor; the common errors' help (a before, highlighted and never drawn,
    and an after), E407/E408 marked internal; an old /dev/psi/#E202 sent on
    to its entry; both under the Psi pages' CSP from _headers (the docs'
    policy and 'wasm-unsafe-eval', set on the pages' responses here too, so a
    violation shows without --base); /psiref.js at the site's root, nothing
    of Psi loaded until an example scrolls near; every live example (a
    cheat-sheet row's, a recipe's) drawn and lit once on screen, its Open in
    editor link opening the demo's editor with that formula; the search
    filtering rows (the reference) and errors (the error page), the cheat
    sheet folding each page; no sideways scroll on a phone. Screenshots of
    them at both sizes in `shots`."""
    csp = csp_for(site, "/dev/psi/*")
    check("'wasm-unsafe-eval'" in csp and "connect-src" not in csp and "blob:" not in csp,
          "psi reference: its CSP is the docs' and 'wasm-unsafe-eval', nothing more")
    check(all((site / f).is_file() for f in ("dev/psi/index.html", "dev/psi/errors/index.html", "psiref.js"))
          and not (site / "dev/psi/psiref.js").exists(),
          "psi reference: dev/psi/index.html, dev/psi/errors/index.html and /psiref.js are in the site")
    check('src="../../psiref.js"' in (site / "dev/psi/index.html").read_text()
          and 'src="../../../psiref.js"' in (site / "dev/psi/errors/index.html").read_text(),
          "psi reference: both pages load the one psiref.js")
    check('href="../dev/psi/"' in (site / "guide/formulas.html").read_text(), "psi reference: linked from every page (the footer)")
    for size in SIZES:
        page = new_page(browser, size, errors)
        violations: list[str] = []
        asked: list[str] = []
        page.on("console", lambda m: "Content Security Policy" in m.text and violations.append(m.text))
        page.on("request", lambda r: asked.append(r.url))

        def with_csp(route):
            r = route.fetch()
            route.fulfill(response=r, headers={**r.headers, "content-security-policy": csp})
        page.route("**/dev/psi/**", with_csp)
        # An error's old address: sent on to the error codes page.
        page.goto(f"{base}/dev/psi/#E202")
        page.wait_for_url(f"{base}/dev/psi/errors/#E202", timeout=10000)
        page.wait_for_selector("h1")
        wait_js(page, "document.querySelector('main').dataset.psiRef == 'ready'")
        check(page.url == f"{base}/dev/psi/errors/#E202", f"psi errors: /dev/psi/#E202 goes on to its entry ({page.url})")
        if size == "laptop":
            page.wait_for_timeout(300)
            check(page.title().startswith("Psi error codes"), f"psi errors: its title ({page.title()})")
            check(page.locator(".side a.on").inner_text() == "Psi error codes" and page.locator(".topnav a.on").inner_text() == "For developers",
                  "psi errors: in the developers' menu, marked")
            codes = page.eval_on_selector_all("h3[id^=E]", "els => els.map(e => e.id)")
            check(len(codes) >= 100 and "E202" in codes and "E601" in codes, f"psi errors: the error index ({len(codes)} codes, each an anchor)")
            top = page.evaluate("document.getElementById('E202').getBoundingClientRect().top")
            check(0 <= top < 400, f"psi errors: #E202 opens at its entry ({top:.0f} px from the top)")
            e202 = page.locator(".psi-err:has(#E202)").inner_text()
            check("Before:" in e202 and "After:" in e202 and "gray(sin(x))" in e202, "psi errors: E202 has a before and an after")
            bad = page.locator(".psi-err:has(#E202) pre.psi-hl code.language-psi-bad").count()
            check(bad == 1 and page.locator(".psi-live").count() == 0, f"psi errors: a before is highlighted ({bad}) and nothing is drawn")
            internal = page.eval_on_selector_all("h3[id^=E]:has(.badge)", "els => els.map(e => e.id)")
            e407 = page.locator(".psi-err:has(#E407)").inner_text()
            e408 = page.locator(".psi-err:has(#E408)").inner_text()
            check(internal == ["E407"] and "a bug in Psi: please report it" in e407 and "not the formula" in e408,
                  f"psi errors: E407 marked internal, E408 explained ({internal})")
            early = [u for u in asked if "pkg-psi" in u]
            check(not early, f"psi errors: Psi never loads there ({early[:1]})")
            page.fill("#psi-q", "cycle")
            page.wait_for_timeout(200)
            errs = page.eval_on_selector_all(".psi-err:not([hidden]) h3", "els => els.map(e => e.id)")
            check("E931" in errs and "E202" not in errs and "error" in page.inner_text("#psi-n") and "built-in" not in page.inner_text("#psi-n"),
                  f"psi errors: search 'cycle' leaves {len(errs)} errors ({page.inner_text('#psi-n')})")
            page.fill("#psi-q", "")
            page.click("#psi-cheat")
            page.wait_for_timeout(200)
            sheet = page.evaluate("""() => ({ blocks: [...document.querySelectorAll('.psi-err blockquote')].some(e => e.checkVisibility()),
                titles: [...document.querySelectorAll('.psi-err h3')].filter(e => e.checkVisibility()).length })""")
            check(not sheet["blocks"] and sheet["titles"] >= 100, f"psi errors: the cheat sheet keeps the error titles only ({sheet})")
            if shots:
                page.evaluate("scrollTo(0, 0)")
                page.screenshot(path=str(shots / "psi-errors-cheat-laptop.png"))
            page.click("#E931")
            page.wait_for_timeout(300)
            check(page.get_attribute("#psi-cheat", "aria-pressed") == "false" and page.locator(".psi-err:has(#E931) blockquote").is_visible()
                  and page.url.endswith("#E931"), "psi errors: an error's title in the cheat sheet opens its entry")
            # The reference.
            asked.clear()
            page.goto(f"{base}/dev/psi/")
            page.wait_for_selector("h1")
            wait_js(page, "document.querySelector('main').dataset.psiRef == 'ready'")
            page.wait_for_timeout(600)
            early = [u for u in asked if "pkg-psi" in u]
            check(not early, f"psi reference: Psi doesn't load with the page, only when an example is near ({early[:1]})")
            check(page.title().startswith("Psi language reference"), f"psi reference: its title ({page.title()})")
            check(page.locator(".side a.on").inner_text() == "Psi language" and page.locator(".topnav a.on").inner_text() == "For developers",
                  "psi reference: in the developers' menu, marked")
            check(page.locator("h3[id^=E]").count() == 0, "psi reference: the error codes are on their own page")
            cells = page.eval_on_selector_all("table code", "els => els.map(e => e.textContent)")
            check(all(any(n in c for c in cells) for n in ("atan2", "rainbow", "screen", "control")),
                  f"psi reference: the generated tables ({len(cells)} names)")
            check(page.locator("pre code", has_text="formula   =").count() == 1, "psi reference: the grammar")
            check(page.locator("pre code", has_text="psi check").count() >= 1, "psi reference: the psi command's example")
            # The name index: its names link the rows.
            ids = page.eval_on_selector_all("table.psi-ex-t tr[id^=n-]", "els => els.map(e => e.id)")
            links = page.eval_on_selector_all("#every-name ~ table a[href^='#n-']", "els => els.map(e => e.getAttribute('href').slice(1))")
            check(len(ids) >= 60 and len(set(ids)) == len(ids) and "n-sin" in ids and len(links) >= 60 and all(i in ids for i in links),
                  f"psi reference: {len(ids)} rows with ids, {len(links)} names in the index linking them")
            page.click("#every-name ~ table a[href='#n-smoothstep']")
            page.wait_for_timeout(300)
            top = page.evaluate("document.getElementById('n-smoothstep').getBoundingClientRect().top")
            check(page.url.endswith("#n-smoothstep") and 0 <= top < 400, f"psi reference: an index name goes to its row ({top:.0f} px)")
            page.evaluate("scrollTo(0, 0)")
            # Every example, scrolled to: drawn and lit.
            n = page.locator(".psi-live").count()
            recipes = page.locator(".psi-live.big").count()
            dark, undrawn, links = [], [], []
            for k in range(n):
                box = page.locator(".psi-live").nth(k)
                box.scroll_into_view_if_needed()
                try:
                    wait_js(page, f"document.querySelectorAll('.psi-live canvas')[{k}].dataset.frames > 0", 10000)
                except PlaywrightTimeout:
                    undrawn.append(k)
                    continue
                # Some examples are dark for part of their loop (a scene that is
                # on half of every 4 s): lit at some point within 5 s.
                lit_js = f"""() => {{ const c = document.querySelectorAll('.psi-live canvas')[{k}];
                    const d = c.getContext('2d').getImageData(0, 0, 128, 32).data; let n = 0;
                    for (let i = 0; i < d.length; i += 4) if (d[i] + d[i+1] + d[i+2] > 90) n++; return n >= 12; }}"""
                try:
                    wait_js(page, lit_js, 5000)
                except PlaywrightTimeout:
                    dark.append(k)
                links.append(box.locator("a.psi-open").get_attribute("href"))
            check(n >= 60 and recipes >= 7 and not undrawn and not dark,
                  f"psi reference: {n} live examples ({recipes} recipes), each drawn ({undrawn[:3]} not) and lit ({dark[:3]} dark)")
            check(all(u == f"{base}/f/#open" for u in links), f"psi reference: every example opens in the site's editor, no code in the link ({links[:1]})")
            moving = page.locator(".psi-live.big canvas").first
            moving.scroll_into_view_if_needed()
            f0 = int(moving.get_attribute("data-frames") or 0)
            page.wait_for_timeout(500)
            check(int(moving.get_attribute("data-frames") or 0) > f0, "psi reference: a recipe on screen moves")
            if shots:
                page.locator("#recipes").scroll_into_view_if_needed()
                page.wait_for_timeout(500)
                page.screenshot(path=str(shots / "psi-reference-recipes-laptop.png"))
                page.locator("#what-each-does").scroll_into_view_if_needed()
                page.wait_for_timeout(500)
                page.screenshot(path=str(shots / "psi-reference-examples-laptop.png"))
            # Search: the rows.
            page.fill("#psi-q", "cycle")
            page.wait_for_timeout(200)
            rows = page.eval_on_selector_all("table.psi-ex-t tbody tr:not([hidden])", "els => els.map(e => e.cells[0].textContent)")
            shown_cook = page.locator("#recipes").is_visible() or page.evaluate(
                "[...document.querySelectorAll('.psi-live.big, #recipes ~ pre')].some(e => e.checkVisibility())")
            check(rows and all("cycle" in page.locator("table.psi-ex-t tbody tr:not([hidden])").nth(i).inner_text() for i in range(len(rows)))
                  and "cycle(fire, v, s)" in rows and not shown_cook
                  and "built-in" in page.inner_text("#psi-n") and "error" not in page.inner_text("#psi-n"),
                  f"psi reference: search 'cycle' leaves {len(rows)} rows, the rest hidden ({page.inner_text('#psi-n')})")
            if shots:
                page.evaluate("scrollTo(0, 0)")
                page.screenshot(path=str(shots / "psi-reference-search-laptop.png"))
            page.fill("#psi-q", "")
            page.click("#psi-cheat")
            page.wait_for_timeout(200)
            sheet = page.evaluate("""() => ({ cook: !!document.querySelector('#recipes')?.checkVisibility(),
                rows: document.querySelectorAll('table.psi-ex-t tbody tr:not([hidden])').length,
                ex: [...document.querySelectorAll('.psi-live')].some(e => e.checkVisibility()) })""")
            check(not sheet["cook"] and sheet["rows"] >= 60 and not sheet["ex"],
                  f"psi reference: the cheat sheet keeps the rows only ({sheet})")
            if shots:
                page.evaluate("scrollTo(0, 0)")
                page.screenshot(path=str(shots / "psi-reference-cheat-laptop.png"))
            page.click("#psi-cheat")
            # Open in editor: the site's editor with that formula (left in this site's storage).
            page.evaluate("scrollTo(0, 0)")
            first = page.locator(".psi-live a.psi-open").first
            first.scroll_into_view_if_needed()
            first.click()
            f_ready(page, 30000)
            stored = page.evaluate("localStorage.getItem('odmd.psi.open')")
            check(page.url == f"{base}/f/#open" and page.input_value("#psi-src").strip() != "" and stored is None,
                  f"psi reference: Open in editor opens the site's editor with it, read once ({page.input_value('#psi-src')[:40]!r})")
            learn = page.eval_on_selector("#psi-learn", "a => a.href")
            check(learn == f"{base}/guide/psi-1.html", f"/f/: the editor's Learn Psi opens the guide's tour on this site ({learn})")
            page.click("#psi-help")
            page.wait_for_selector("#psi-cheat a[href$='guide/psi-1.html']", timeout=10000)
            foot = page.eval_on_selector("#psi-cheat a[href$='guide/psi-1.html']", "a => a.href")
            check(foot == f"{base}/guide/psi-1.html", f"/f/: the cheat sheet's foot links the tour ({foot})")
        else:
            page.wait_for_timeout(300)
            check(page.evaluate("document.documentElement.scrollWidth <= innerWidth"), "psi errors (phone): no sideways scroll")
            page.goto(f"{base}/dev/psi/")
            wait_js(page, "document.querySelector('main').dataset.psiRef == 'ready'")
            page.wait_for_timeout(300)
            check(page.evaluate("document.documentElement.scrollWidth <= innerWidth"), "psi reference (phone): no sideways scroll")
            wide = page.evaluate("[...document.querySelectorAll('main pre:has(code.language-text)')].filter(p => p.scrollWidth > p.clientWidth + 1 && !p.closest('[hidden]')).map(p => p.textContent.slice(0, 30))")
            check(not wide, f"psi reference (phone): the grammar and the psi transcript don't scroll sideways {wide[:2]}")
            page.locator("#what-each-does").scroll_into_view_if_needed()
            wait_js(page, "[...document.querySelectorAll('.psi-live canvas')].some(c => c.dataset.frames > 0)", 15000)
            box = page.locator(".psi-live .led").first.bounding_box()
            check(page.evaluate("document.documentElement.scrollWidth <= innerWidth") and box and box["x"] + box["width"] <= 390,
                  "psi reference (phone): the examples fit, no sideways scroll")
            if shots:
                page.wait_for_timeout(400)
                page.screenshot(path=str(shots / "psi-reference-examples-phone.png"))
                page.locator("#recipes").scroll_into_view_if_needed()
                page.wait_for_timeout(600)
                page.screenshot(path=str(shots / "psi-reference-recipes-phone.png"))
        check(not violations, f"psi reference [{size}]: no CSP violations {violations[:2]}")
        page.context.close()


def psi_error_links(browser, base: str, errors: list[str]) -> None:
    """An error's code links its entry on the error codes page, on the site's
    own copy: /f/'s editor (a broken formula) and the demo's; the entry is
    there."""
    page = new_page(browser, "laptop", errors)
    open_text(page, base, "gray(sine(x))", "Broken")
    page.wait_for_selector("#psi-err a.ecode", timeout=20000)
    a = page.locator("#psi-err a.ecode")
    href = a.get_attribute("href")
    check(a.inner_text() == "E202" and href == f"{base}/dev/psi/errors/#E202" and "Line 1" in page.inner_text("#psi-err"),
          f"/f/: a broken formula's code links the reference ({href})")
    page.goto(href)
    page.wait_for_selector("#E202")
    check(page.url == f"{base}/dev/psi/errors/#E202", f"/f/: the code's link leads to its entry ({page.url})")
    page.goto(f"{base}/demo/#formulas")
    page.wait_for_selector("#psi-ex .psi-tile", timeout=30000)
    page.click("#psi-new")
    page.wait_for_selector("#psi-ed #psi-src", timeout=10000)
    page.fill("#psi-src", "gray(tme)")
    page.wait_for_selector("#psi-err a.ecode", timeout=10000)
    href = page.get_attribute("#psi-err a.ecode", "href")
    check(href == f"{base}/dev/psi/errors/#E201", f"demo: the editor's error code links the site's own reference ({href})")
    learn = page.eval_on_selector("#psi-learn", "a => a.href")
    check(learn == f"{base}/guide/psi-1.html", f"demo: the editor's Learn Psi opens the guide's tour on this site ({learn})")
    back = page.eval_on_selector_all("#top .demo-site a", "els => els.map(a => [a.textContent, a.getAttribute('href')])")
    check(back == [["‹ opendmd.dev", "../"], ["Install", "../install/"]] and page.is_visible("#top .demo-site"),
          f"demo: its header leads back to the site and to Install ({back})")
    page.context.close()


def f_cost(browser, base: str, errors: list[str], shots: Path) -> None:
    """The site editor's What costs most and Tidy up (js/fcost.js, its wasm
    pkg-psi-lang): a heavy Formula gets a bar in the gutter beside its
    costly lines and the compiler's notes with Line buttons that move the
    caret; Tidy up lays the text out as `psi fmt` does and one undo takes it
    back; a light one stays quiet until What costs most is pressed; the
    demo's editor has Tidy up too."""
    src = 'name "Heavy"\nloop 4\na = noise(x*8, y*8, t)\nb = noise(x*4 + a,  y*4, t/2)\nc = sqrt(x*x + y*y) + atan2(y - 0.5, x - 0.5)\ngray(frac(a + b + c))'
    page = new_page(browser, "laptop", errors)
    open_text(page, base, src, "Heavy")
    page.wait_for_selector(".psi-gut .fc-bar", timeout=20000)
    bars, notes = page.locator(".psi-gut .fc-bar").count(), page.inner_text("#psi-notes")
    check(bars >= 3 and "What costs most" in notes and "Line 4" in notes and "it reads t, which changes" in notes,
          f"/f/: a heavy Formula's lines have cost bars ({bars}) and its notes are said ({notes[:80]!r}…)")
    check(not re.search(r"cache|copy|bucket", notes, re.I), "/f/: the notes say no cache, copy or bucket")
    print(f"  (observe() in the page: {page.get_attribute('#psi-notes', 'data-ms')} ms)")
    page.locator("#psi-notes button", has_text="Line 4").first.click()
    check(page.evaluate("document.activeElement.id === 'psi-src' && document.querySelector('#psi-src').selectionStart") == src.index("b = "),
          "/f/: a note's Line button puts the caret on its line")
    shots.mkdir(parents=True, exist_ok=True)
    page.locator("#psi-ed").screenshot(path=str(shots / "f-cost.png"))
    page.click("#psi-tidy")
    wait_js(page, "!document.querySelector('#psi-src').value.includes('a,  y')", timeout=20000)
    check(page.input_value("#psi-src") == src.replace("a,  y", "a, y") + "\n", "/f/: Tidy up lays the text out")
    page.keyboard.press("Control+z")
    check(page.input_value("#psi-src") == src, "/f/: one undo takes Tidy up back")
    page.context.close()
    page = new_page(browser, "laptop", errors)
    road = next(e for e in examples(True) if e["slug"] == "endless-road")
    open_text(page, base, road["src"], road["name"])
    page.wait_for_selector("#psi-tidy:not([hidden])", timeout=20000)
    page.wait_for_timeout(800)
    check(page.locator(".psi-gut .fc-bar").count() == 0 and not page.is_visible("#psi-notes"), "/f/: a light Formula: no bars, no notes by themselves")
    page.click("#psi-cost")
    page.wait_for_selector(".psi-gut .fc-bar", timeout=20000)
    notes = page.inner_text("#psi-notes")
    check(page.get_attribute("#psi-cost", "aria-pressed") == "true" and notes.startswith("Light for the panel") and "What costs most" in notes,
          f"/f/: What costs most on a light bundled Formula ({notes[:90]!r}…)")
    page.locator("#psi-ed").screenshot(path=str(shots / "f-cost-light.png"))
    page.click("#psi-tidy")
    page.wait_for_selector("#toasts .toast", timeout=10000)
    check("Already tidy" in page.inner_text("#toasts"), "/f/: a bundled Formula is already tidy")
    page.click("#psi-cost")
    page.wait_for_timeout(300)
    check(page.locator(".psi-gut .fc-bar").count() == 0 and not page.is_visible("#psi-notes"), "/f/: pressed again, What costs most goes")
    page.goto(f"{base}/demo/#formulas")
    page.wait_for_selector("#psi-ex .psi-tile", timeout=30000)
    page.click("#psi-new")
    page.wait_for_selector("#psi-ed #psi-tidy:not([hidden])", timeout=10000)
    check(True, "demo: the editor has Tidy up")
    page.context.close()


def changelog(site: Path) -> None:
    """The changelog page and its feed (tools/site.py `changelog`), against
    the releases this site serves (install/release.json, install/beta.json)."""
    text = (site / "changelog/index.html").read_text(encoding="utf-8")
    final = json.loads((site / "install/release.json").read_text())["version"]
    newest = json.loads((site / "install/beta.json").read_text())["version"]

    def key(v: str) -> tuple:  # 1.0.0-beta.2 < 1.0.0-beta.10 < 1.0.0
        core, _, pre = v.partition("-")
        return tuple(int(x) for x in core.split(".")), not pre, tuple(int(x) if x.isdigit() else x for x in pre.split("."))
    vers = re.findall(r'<(?:section|article) class="rel(?:-row)?" id="v(\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?)"', text)
    check(bool(vers) and vers == sorted(vers, key=key, reverse=True) and len(set(vers)) == len(vers),
          f"changelog: {len(vers)} releases, newest first ({vers[0] if vers else 'none'} down to {vers[-1] if vers else 'none'})")
    # A release with no entry yet is not a failure (a release never waits for its highlights).
    check(bool(vers) and key(vers[0]) <= key(newest),
          f"changelog: nothing newer than the newest release the site serves ({vers[:1]}, {newest})")
    for v in dict.fromkeys((newest, final)):
        if v not in vers:
            print(f"  NOTE  the changelog has no entry for {v} (site/pages/changelog.md)")
    # A tag with no GitHub release (tools/site.py CHANGELOG_TAG_ONLY) links its tree, never a release page.
    gh, tag_only = "https://github.com/opendmd/opendmd/", {"0.3.0"}
    bad = [v for v in vers if (f'href="{gh}tree/v{v}"' in text, f'href="{gh}releases/tag/v{v}"' in text) != (v in tag_only, v not in tag_only)]
    check(not bad, f"changelog: every release linked to its GitHub release, a tag without one ({sorted(tag_only & set(vers))}) to its tree {bad}")
    tagged = dict(re.findall(r'<section class="rel" id="v(\d[^"]*)"><p class="rel-meta"><time[^>]*>[^<]*</time>(?:<span>(\w+)</span>)?', text))
    want = {v: "Beta" if "-" in v else "Latest" if v == final else "" for v in tagged}
    check(tagged == want and tagged, f"changelog: pre-releases marked Beta, {final} marked Latest "
                                     f"({ {v: t for v, t in tagged.items() if t != want[v]} or 'as built'})")
    feed = (site / "changelog/feed.xml").read_text(encoding="utf-8")
    import xml.etree.ElementTree as ET
    ns = {"a": "http://www.w3.org/2005/Atom"}
    ids = [e.findtext("a:id", namespaces=ns) for e in ET.fromstring(feed).findall("a:entry", ns)]
    check(ids == [f"https://opendmd.dev/changelog/#v{v}" for v in vers] and 'href="../' not in feed and "feed.xml" in text,
          f"changelog: the Atom feed has the same {len(ids)} entries, its links whole, and the page names it")
    check('href="../changelog/"' in (site / "guide/index.html").read_text() and f'href="../changelog/#v{final}"' in (site / "install/index.html").read_text(),
          "changelog: linked from every page's footer and from the install page's version line")


def install_links(site: Path, dist: Path, beta_dist: Path | None) -> None:
    install = (site / "install/index.html").read_text()
    fw = site / "install/firmware"
    rel_info = json.loads((site / "install/release.json").read_text())
    beta_info = json.loads((site / "install/beta.json").read_text())
    ver = json.loads((dist / "manifest-4mb.json").read_text())["version"]
    # One board: the DevKit's button (and, with a beta newer than the release,
    # the beta's below it); nothing for another layout.
    buttons = 2 if beta_dist else 1
    check(install.count("<esp-web-install-button") == buttons and 'manifest="firmware/manifest-4mb.json"' in install
          and "8mb" not in install and "8 MB" not in install and "Not sure which" not in install,
          f"install: {buttons} Install button(s), the DevKit's, and nothing for another layout")
    check(rel_info["version"] == ver and filecmp.cmp(fw / "manifest-4mb.json", dist / "manifest-4mb.json", shallow=False)
          and f"OpenDMD {ver}." in install,
          f"install: release.json and the Install button are --dist's release ({rel_info['version']}, {ver})")
    # beta.json: the same shape, the newest release. Without a beta newer than
    # the release it is release.json byte for byte, and there is no beta folder.
    bdir = site / "install/firmware/beta"
    if beta_dist is None:
        check((site / "install/beta.json").read_bytes() == (site / "install/release.json").read_bytes()
              and not bdir.exists() and "beta-install" not in install,
              "install: beta.json is release.json (no beta newer than the release), no beta button")
    else:
        bver = json.loads((beta_dist / "manifest-4mb.json").read_text())["version"]
        files = [beta_info["firmware"], beta_info["www"], *(v["firmware"] for v in beta_info["variants"].values())]
        check(beta_info["version"] == bver and set(beta_info) == set(rel_info) and set(beta_info["variants"]) == set(rel_info["variants"])
              and all(f.startswith("firmware/beta/") and (site / "install" / f).is_file() for f in files)
              and filecmp.cmp(bdir / f"opendmd-{bver}-4mb.odfw", beta_dist / f"opendmd-{bver}-4mb.odfw", shallow=False),
              f"install: beta.json names --beta-dist's {bver}, its files under firmware/beta/ ({beta_info['version']})")
        check(filecmp.cmp(bdir / "manifest-4mb.json", beta_dist / "manifest-4mb.json", shallow=False)
              and all((bdir / pt["path"]).is_file() for b in json.loads((bdir / "manifest-4mb.json").read_text())["builds"] for pt in b["parts"])
              and 'manifest="firmware/beta/manifest-4mb.json"' in install and f"install the beta, OpenDMD {bver}" in install
              and not any("8mb" in p.name for p in bdir.iterdir()),
              "install: the beta's own button, its manifest and parts under firmware/beta/ (the 4mb only)")
    offline_published(site, dist, "firmware")
    if beta_dist is not None:
        offline_published(site, beta_dist, "firmware/beta")
    check((fw / "manifest-4mb.json").is_file() and (fw / "manifest.json").is_file() and not any("8mb" in p.name for p in fw.iterdir()),
          "install: the 4mb manifest is published, no 8mb file (a dist from 1.0.0-beta.16 has them)")
    check(all((site / "install" / v["firmware"]).is_file() for v in rel_info["variants"].values()) and set(rel_info["variants"]) == {"4mb"},
          "install: release.json's one variant (4mb) and its bundle are published")
    # Each variant names the partition table its bundle is for (the web app
    # tells a player on another table to reinstall over USB): the bundle's magic.
    magics = {("4mb", 4): b"ODFW44mb", ("4mb", 3): b"ODFW34mb", ("4mb", 2): b"ODFW1\0\0\0"}
    lay = {k: (v.get("layout"), (site / "install" / v["firmware"]).read_bytes()[:8]) for k, v in rel_info["variants"].items()}
    # (v3 for a site built from a release before table v4, 4 after.)
    check(all(magics.get((k, l)) == m for k, (l, m) in lay.items()) and lay["4mb"][0] in (3, 4),
          f"install: release.json's layouts are the bundles' ({ {k: l for k, (l, _) in lay.items()} })")


# What the offline box's card file holds (tools/site.py OFFLINE_FILE).
OFFLINE_FILE = b'{"network":"off"}\n'


def offline_published(site: Path, src: Path, sub: str) -> bool:
    """`src`'s offline install (when its dist has one) under install/<sub>/,
    byte for byte with its parts, and its button's box on the page; False
    when `src` has none (a release from before it)."""
    install = (site / "install/index.html").read_text()
    fw = site / "install" / sub
    m = src / "manifest-4mb-offline.json"
    bid = "install-4mb" if sub == "firmware" else "install-beta-4mb"
    if not m.is_file():
        check(not (fw / m.name).exists() and f'data-offline-for="{bid}"' not in install,
              f"install: {src} has no offline install: none under install/{sub}/, no box for {bid}")
        return False
    om = json.loads(m.read_text())
    parts = om["builds"][0]["parts"]
    check(filecmp.cmp(fw / m.name, m, shallow=False) and len(parts) == 7
          and all(filecmp.cmp(fw / pt["path"], src / pt["path"], shallow=False) for pt in parts)
          and filecmp.cmp(fw / "nvs-offline-4mb.bin", src / "nvs-offline-4mb.bin", shallow=False),
          f"install: {sub}/manifest-4mb-offline.json and its seven parts (nvs-offline-4mb.bin among them) are {src}'s")
    check(f'data-manifest-offline="{sub}/manifest-4mb-offline.json"' in install and f'data-offline-for="{bid}"' in install,
          f"install: {bid} has its offline box")
    return True


def install_page(browser, base: str, site: Path, dist: Path, beta_dist: Path | None, errors: list[str]) -> None:
    """The install page in Chromium under its CSP: each offline box swaps its
    button's manifest and back, its card file downloads, and the offline
    manifest and NVS image are served."""
    page = new_page(browser, "laptop", errors)
    console: list[str] = []
    page.on("console", lambda m: m.type == "error" and console.append(m.text[:160]))
    page.on("pageerror", lambda e: console.append(str(e)[:160]))

    def with_csp(route):
        r = route.fetch()
        route.fulfill(response=r, headers={**r.headers, "content-security-policy": csp_for(site, "/install/*")})
    page.route(lambda u: urllib.parse.urlsplit(u).path.endswith(("/", ".html")), with_csp)
    page.goto(base + "/install/", wait_until="load")
    wait_js(page, "customElements.get('esp-web-install-button') !== undefined", timeout=30000)
    for src, sub, bid in ((dist, "firmware", "4mb"), (beta_dist, "firmware/beta", "beta-4mb")):
        if src is None or not (src / "manifest-4mb-offline.json").is_file():
            continue
        if sub != "firmware":
            page.click("#beta-install summary")
        box = page.locator(f"#offline-{bid}")
        label = page.locator(f"label:has(#offline-{bid})").inner_text().strip()
        check(box.is_visible() and not box.is_checked() and label == "Play from the card only, no WiFi"
              and page.get_attribute(f"#install-{bid}", "manifest") == f"{sub}/manifest-4mb.json",
              f"install: the {bid} box, labelled {label!r}, unticked, the button on {sub}/manifest-4mb.json")
        box.check()
        on = page.get_attribute(f"#install-{bid}", "manifest")
        box.uncheck()
        off = page.get_attribute(f"#install-{bid}", "manifest")
        check(on == f"{sub}/manifest-4mb-offline.json" and off == f"{sub}/manifest-4mb.json",
              f"install: ticked, the {bid} button reads {on}; unticked, {off}")
        about = page.inner_text(f"#offline-about-{bid}")
        check(all(w in about for w in ("erases", "Connect to Wi-Fi", "unticked", "opendmd.json", '"on"'))
              and not HIDDEN_WORDS.search(about) and not any(w in about for w in ("NVS", "partition", "player")),
              f"install: the {bid} box says what it does and how to undo it, in plain words")
        with page.expect_download() as dl:
            page.click(f"#offline-file-{bid}")
        got = Path(dl.value.path()).read_bytes()
        check(dl.value.suggested_filename == "opendmd.json" and got == OFFLINE_FILE,
              f"install: the {bid} card file downloads as {dl.value.suggested_filename}: {got!r}")
        for name in ("manifest-4mb-offline.json", "nvs-offline-4mb.bin"):
            r = page.request.get(f"{base}/install/{sub}/{name}")
            check(r.status == 200 and r.body() == (src / name).read_bytes(), f"install: served {sub}/{name} ({len(r.body()):,} B)")
    check(not console, f"install: no console errors {console[:2]}")
    page.context.close()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("site", type=Path)
    ap.add_argument("--shots", type=Path)
    ap.add_argument("--gallery-tool", type=Path, default=ROOT / "opendmd-gallery/target/release/opendmd-gallery")
    ap.add_argument("--dist", type=Path, default=ROOT / "dist", help="the release make site published (its manifest's version)")
    ap.add_argument("--beta-dist", type=Path, help="the beta make site published beside it (BETA_DIST), if any")
    ap.add_argument("--art", type=Path, default=ROOT / "target/site-art", help="tools/site-art.py's output")
    ap.add_argument("--base", help="fetch the site from here (e.g. cf dev's http://127.0.0.1:8787) instead of serving SITE")
    ap.add_argument("--only", choices=["studio"], help="run only these checks (the Studio's), e.g. against an older build")
    ap.add_argument("--fn", help="run only these check functions, by name, comma-separated (a red run against an older build)")
    a = ap.parse_args()
    (a.shots or ROOT / "target/site-smoke").mkdir(parents=True, exist_ok=True)
    base = a.base.rstrip("/") if a.base else serve(a.site)
    print(f"site: {base}{' (' + str(a.site) + ', served here)' if not a.base else ''}")
    errors: list[str] = []
    tool = a.gallery_tool if a.gallery_tool.is_file() else None
    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True)
        if a.fn:
            for name in a.fn.split(","):
                fn = globals()[name]
                params = list(__import__("inspect").signature(fn).parameters)
                args = {"browser": browser, "base": base, "site": a.site, "errors": errors, "shots": a.shots or ROOT / "target/site-smoke"}
                print(name)
                try:
                    fn(*[args[x] for x in params if x in args])
                except Exception as e:  # (a red run: a check that cannot even finish fails, and the rest still run)
                    check(False, f"{name} did not finish: {str(e).splitlines()[0][:160]}")
                    for c in browser.contexts:
                        c.close()
            browser.close()
            return finish(errors)
        if a.only == "studio":
            print("the Studio (/studio/)")
            studio_page(browser, base, a.site, errors, a.shots)
            studio_tabs(browser, base, a.site, errors)
            studio_tools(browser, base, a.site, errors)
            studio_colours(browser, base, a.site, errors, a.shots)
            studio_journal(browser, base, a.site, errors)
            studio_rename_tabs(browser, base, a.site, errors)
            studio_edges(browser, base, a.site, errors)
            studio_phone(browser, base, a.site, errors)
            studio_fetches(browser, base, a.site, errors)
            studio_layers(browser, base, a.site, errors)
            studio_old_journal(browser, base, a.site, errors)
            studio_shade(browser, base, a.site, errors)
            studio_two_keys(browser, base, a.site, errors)
            studio_model(browser, base, a.site, errors)
            studio_bombs(browser, base, a.site, errors)
            studio_records(browser, base, a.site, errors)
            studio_journals(browser, base, a.site, errors)
            studio_budget(a.site)
            studio_paste_redo(browser, base, a.site, errors)
            studio_keyless_restore(browser, base, a.site, errors)
            studio_bad_palette(browser, base, a.site, errors)
            studio_orphan_keys(browser, base, a.site, errors)
            studio_lost_steps(browser, base, a.site, errors)
            studio_garbage_drafts(browser, base, a.site, errors)
            studio_starter_times(a.site)
            studio_frames(browser, base, a.site, errors)
            studio_frames_out(browser, base, a.site, errors)
            studio_send(browser, base, a.site, errors)
            studio_send_panel(browser, base, a.site, errors)
            studio_phone_frames(browser, base, a.site, errors)
            studio_frames_red(browser, base, a.site, errors)
            studio_shape_journals(browser, base, a.site, errors)
            studio_midstroke(browser, base, a.site, errors)
            studio_wide_shape(browser, base, a.site, errors)
            studio_onion_bg(browser, base, a.site, errors)
            studio_send_sheet(browser, base, a.site, errors)
            studio_journal_z(browser, base, a.site, errors)
            studio_small_things(browser, base, a.site, errors)
            studio_layout(browser, base, a.site, errors)
            studio_library(browser, base, a.site, errors)
            studio_files(browser, base, a.site, errors)
            studio_stamps(browser, base, a.site, errors)
            studio_files_hostile(browser, base, a.site, errors)
            studio_starter_bad(browser, base, a.site, errors)
            studio_open_cost(browser, base, a.site, errors)
            studio_files_trust(browser, base, a.site, errors)
            studio_files_more(browser, base, a.site, errors)
            studio_backup_limits(browser, base, a.site, errors)
            studio_backup_parts(browser, base, a.site, errors)
            studio_read_live(browser, base, a.site, errors)
            studio_files_attacks(browser, base, a.site, errors)
            studio_open_limits(browser, base, a.site, errors)
            studio_open_limits2(browser, base, a.site, errors)
            studio_held_members(browser, base, a.site, errors)
            studio_import_fetches(browser, base, a.site, errors)
            studio_import(browser, base, a.site, errors, a.shots)
            studio_import_video(browser, base, a.site, errors)
            studio_import_paste(browser, base, a.site, errors)
            studio_import_handoff(browser, base, a.site, errors)
            studio_import_phone(browser, base, a.site, errors, a.shots)
            studio_import_budget(a.site)
            studio_import_red(browser, base, a.site, errors)
            studio_paste_red(browser, base, a.site, errors)
            studio_import_video_red(browser, base, a.site, errors)
            studio_import_history(browser, base, a.site, errors)
            studio_import_review(browser, base, a.site, errors)
            studio_links(browser, base, a.site, errors, a.shots)
            studio_notices(browser, base, a.site, errors, a.shots)
            studio_links_red(browser, base, a.site, errors)
            studio_links_leftovers(browser, base, a.site, errors)
            studio_links_review(browser, base, a.site, errors)
            studio_tidy(browser, base, a.site, errors)
            studio_import_colours(browser, base, a.site, errors)
            studio_tidy_review(browser, base, a.site, errors)
            browser.close()
            return finish(errors)
        print("the site's structure (menu, home, themes, phone width, CSP, every page reachable, old addresses)")
        structure(browser, base, a.site, errors)
        print("the home page's hero show")
        hero_show(browser, base, errors)
        print("the home page's scenes (?scene=)")
        scenes(browser, base, a.site, errors)
        print("formula page (/f/)")
        f_page(browser, base, errors, tool, a.shots)
        f_drafts(browser, base, errors, a.shots)
        print("Create Art (/create/)")
        create_page(browser, base, a.site, errors, a.shots)
        create_send(browser, base, a.site, errors)
        create_links(browser, base, a.site, errors)
        create_video(browser, base, a.site, errors, a.shots or ROOT / "target/site-smoke")
        create_elsewhere(browser, base, a.site, errors, a.shots or ROOT / "target/site-smoke")
        print("the Studio (/studio/)")
        studio_page(browser, base, a.site, errors, a.shots)
        studio_tabs(browser, base, a.site, errors)
        studio_tools(browser, base, a.site, errors)
        studio_colours(browser, base, a.site, errors, a.shots)
        studio_journal(browser, base, a.site, errors)
        studio_rename_tabs(browser, base, a.site, errors)
        studio_edges(browser, base, a.site, errors)
        studio_phone(browser, base, a.site, errors)
        studio_fetches(browser, base, a.site, errors)
        studio_layers(browser, base, a.site, errors)
        studio_old_journal(browser, base, a.site, errors)
        studio_shade(browser, base, a.site, errors)
        studio_two_keys(browser, base, a.site, errors)
        studio_model(browser, base, a.site, errors)
        studio_bombs(browser, base, a.site, errors)
        studio_records(browser, base, a.site, errors)
        studio_journals(browser, base, a.site, errors)
        studio_budget(a.site)
        studio_paste_redo(browser, base, a.site, errors)
        studio_keyless_restore(browser, base, a.site, errors)
        studio_bad_palette(browser, base, a.site, errors)
        studio_orphan_keys(browser, base, a.site, errors)
        studio_lost_steps(browser, base, a.site, errors)
        studio_garbage_drafts(browser, base, a.site, errors)
        studio_starter_times(a.site)
        studio_frames(browser, base, a.site, errors)
        studio_frames_out(browser, base, a.site, errors)
        studio_send(browser, base, a.site, errors)
        studio_send_panel(browser, base, a.site, errors)
        studio_phone_frames(browser, base, a.site, errors)
        studio_frames_red(browser, base, a.site, errors)
        studio_shape_journals(browser, base, a.site, errors)
        studio_midstroke(browser, base, a.site, errors)
        studio_wide_shape(browser, base, a.site, errors)
        studio_onion_bg(browser, base, a.site, errors)
        studio_send_sheet(browser, base, a.site, errors)
        studio_journal_z(browser, base, a.site, errors)
        studio_small_things(browser, base, a.site, errors)
        studio_layout(browser, base, a.site, errors)
        studio_library(browser, base, a.site, errors)
        studio_files(browser, base, a.site, errors)
        studio_stamps(browser, base, a.site, errors)
        studio_files_hostile(browser, base, a.site, errors)
        studio_starter_bad(browser, base, a.site, errors)
        studio_open_cost(browser, base, a.site, errors)
        studio_files_trust(browser, base, a.site, errors)
        studio_files_more(browser, base, a.site, errors)
        studio_backup_limits(browser, base, a.site, errors)
        studio_backup_parts(browser, base, a.site, errors)
        studio_read_live(browser, base, a.site, errors)
        studio_files_attacks(browser, base, a.site, errors)
        studio_open_limits(browser, base, a.site, errors)
        studio_open_limits2(browser, base, a.site, errors)
        studio_held_members(browser, base, a.site, errors)
        studio_import_fetches(browser, base, a.site, errors)
        studio_import(browser, base, a.site, errors, a.shots)
        studio_import_video(browser, base, a.site, errors)
        studio_import_paste(browser, base, a.site, errors)
        studio_import_handoff(browser, base, a.site, errors)
        studio_import_phone(browser, base, a.site, errors, a.shots)
        studio_import_budget(a.site)
        studio_import_red(browser, base, a.site, errors)
        studio_paste_red(browser, base, a.site, errors)
        studio_import_video_red(browser, base, a.site, errors)
        studio_import_history(browser, base, a.site, errors)
        studio_import_review(browser, base, a.site, errors)
        studio_links(browser, base, a.site, errors, a.shots)
        studio_notices(browser, base, a.site, errors, a.shots)
        studio_links_red(browser, base, a.site, errors)
        studio_links_leftovers(browser, base, a.site, errors)
        studio_links_review(browser, base, a.site, errors)
        studio_tidy(browser, base, a.site, errors)
        studio_import_colours(browser, base, a.site, errors)
        studio_tidy_review(browser, base, a.site, errors)
        print("the demo's Settings")
        demo_settings(browser, base, errors, a.shots or ROOT / "target/site-smoke")
        demo_own_card(browser, base, a.site, errors)
        print("the demo's Copy code and Submit")
        demo_share(browser, base, errors)
        print("the art index (/art-index/)")
        art_index(browser, base, a.site, errors, cors_expected=bool(a.base))
        print("the gallery (/formulas/)")
        gallery(browser, base, errors, a.shots)
        print("sorting and filtering the gallery")
        gallery_sort(browser, base, errors)
        print("Open on my panel (the gallery's tiles)")
        open_on_panel(browser, base, errors)
        print("Open on my panel, the site to the panel's app (saved, kept over a reload)")
        panel_end_to_end(browser, base, errors)
        print("the HTTP API (dev/api.html, dev/api/)")
        api_pages(browser, a.site, a.dist, a.beta_dist, a.art, errors)
        print("\"On this page\" on the long pages")
        on_this_page(a.site)
        print("the Psi language reference and its error codes (dev/psi/, dev/psi/errors/)")
        psi_reference(browser, base, a.site, errors, a.shots)
        print("error codes linked to the reference (/f/, the demo)")
        psi_error_links(browser, base, errors)
        print("What costs most and Tidy up (/f/, the demo)")
        f_cost(browser, base, errors, a.shots or ROOT / "target/site-smoke")
        print("the changelog")
        changelog(a.site)
        print("install page")
        install_links(a.site, a.dist, a.beta_dist)
        install_page(browser, base, a.site, a.dist, a.beta_dist, errors)
        if a.shots:
            screenshots(browser, base, errors, a.shots)
        browser.close()
    return finish(errors)


def finish(errors: list[str]) -> int:
    for e in errors:
        print(f"  FAIL {e}")
    failures.extend(errors)
    print(f"site-smoke: {'ok' if not failures else f'{len(failures)} failures'}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
