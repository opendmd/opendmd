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
  downloaded with the motion kept;
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
  with `--api-freeze auto` off a tag, nothing new is frozen;
- /dev/psi/ (the Psi language reference): in the menu, its generated
  tables, grammar and error index (every code an anchor, the common ones
  with a before and an after, E407/E408 internal), under its own CSP (the
  docs' and 'wasm-unsafe-eval') with no violation; Psi loaded only when an
  example is near, then every live example drawn and lit, Open in editor
  opening the site's editor (/f/#open, the example never in the link); the search and the cheat sheet; no sideways
  scroll on a phone;
- an error's code linked to the reference: /f/'s editor and the demo's;
- the demo's Settings, against its player in the page (js/demo.js): every
  tab; brightness, the colour, the Formula look, each GIF's stay, the
  playlist, night mode and a rule shown on its panel and in Now's banner;
  panel timing tried and kept, a new name, a key; firmware, WiFi and
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
    # (Create Art's relay refusals and the hosts that refuse the browser, stubbed: on purpose.)
    page.on("console", lambda m: m.type == "error" and not any(a in m.location.get("url", "") for a in ("/api/status", "/formulas/src/no-such-formula", "/api/fetch", "nocors.example"))
            and errors.append(f"[{size}] console: {m.text} ({m.location.get('url')})"))
    page.on("pageerror", lambda e: errors.append(f"[{size}] pageerror: {e}"))
    page.on("response", lambda r: r.status >= 400 and "/demo/api/" not in r.url and "/api/status" not in r.url
            and "/formulas/src/no-such-formula" not in r.url and "/api/fetch" not in r.url and errors.append(f"[{size}] HTTP {r.status} {r.url}"))
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


def panel_route(ctx, seen: list[str]) -> None:
    ctx.route("http://192.168.1.50/**", lambda r: (seen.append(r.request.url), r.fulfill(body=PANEL_STUB, content_type="text/html")))


def sent(popup: Page) -> dict:
    """What the stand-in panel got: {origin, data}."""
    popup.wait_for_function("document.title !== 'waiting'", timeout=20000)
    return json.loads(popup.title())


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
            page.wait_for_function("document.querySelector('#f-name').textContent === 'Edited rings'", timeout=5000)
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

    def relay(r):
        asked.append(f"{r.request.method} {r.request.url} {r.request.post_data}")
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
    for why, status in (("size", 413), ("type", 415), ("rate", 429), ("upstream", 502), ("redirects", 502)):
        answer.update(status=status, why=why)
        if page.is_hidden("#st-url"):  # (Find stays open after a refusal)
            page.click("#st-change")
        page.fill("#st-url", f"https://nocors.example/{why}.gif")
        page.click("#st-url-go")
        wait_js(page, "!/^Loading/.test(document.querySelector('#st-msg').textContent)", timeout=10000)
        check(page.inner_text("#st-msg") == C_WHY[why], f"/create/: the relay's '{why}' said plainly ('{page.inner_text('#st-msg')}')")
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
    page.route(f"{base}/api/fetch", lambda r: r.fulfill(status=200, body=mp4, headers={"content-type": "video/mp4", "x-relayed": "1"}))
    page.goto(f"{base}/create/")
    page.wait_for_selector("#create:not([aria-busy]) #studio .st-grid", timeout=30000)
    check(not any(p.endswith("/create/video.js") for p in asked), "/create/ video: the video reader not loaded with the page")
    page.click("#st-src [data-v=own]")
    accept = page.get_attribute("#st-file", "accept")
    check(accept == "image/*,video/mp4,video/webm" and "video" in page.inner_text("#st-own"), f"/create/ video: Choose takes videos ({accept})")
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
    www = ROOT / "opendmd-web/www"
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
            page.wait_for_function("document.querySelector('#f-name').textContent === 'Site rings'", timeout=5000)
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
    tab("play")
    check(page.locator("text=Device settings, WiFi and reboot live on the player").count() == 0
          and all(page.locator(s).count() == 1 for s in ("#st-brightness", "#st-maxbright", "#st-set", "#st-plat-btn", "#st-dwell-min", "#st-dwell-max", "#st-shuffle")),
          "demo settings: Playback has the player's controls, not the local-mode sentence")
    page.evaluate("(e => { e.value = 30; e.dispatchEvent(new Event('input')); })(document.querySelector('#st-brightness'))")
    saved("#st-brightness")
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
    page.select_option("#sc-set", "Sprites and Clock")
    for d in page.query_selector_all("#sc-days .chip:not(.on)"):
        d.click()
    page.fill("#sc-from", "00:00")
    page.fill("#sc-to", "00:00")
    page.click("#sc-rule-save")
    page.wait_for_selector("#sc-rules [data-rule='0']", timeout=10000)
    now()
    rule = page.inner_text("#now-banner") if page.is_visible("#now-banner") else ""
    check(rule.startswith("Schedule: Sprites and Clock") and page.input_value("#sim-list") == "/sets/Sprites and Clock.txt",
          f"demo settings: a rule's playlist plays, with Now's banner ('{rule}')")
    tab("time")
    page.click("#sc-rules [data-rule='0'] button:has-text('Delete')")
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
    for t in ("play", "look", "time", "device"):
        tab(t)
        page.wait_for_timeout(600)
        page.screenshot(path=str(shots / f"demo-settings-{t}-laptop.png"), full_page=True)
    page.context.close()
    # A new session starts afresh; every tab on a phone.
    page = new_page(browser, "phone", errors)
    page.goto(f"{base}/demo/#settings/look")
    page.wait_for_selector("#st-tab-look", timeout=30000)
    check(page.input_value("#st-look") == "colour", "demo settings: a new session starts with a new player's settings")
    for t in ("play", "look", "time", "device"):
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
            check(page.locator(".topnav a.on").inner_text() == "Formulas", "gallery: Formulas marked in the top bar")
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
        page.wait_for_function("location.search.includes('controls=1')", timeout=3000)
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
    "formulas": "/formulas/", "create": "/create/", "dev": "/dev/", "dev-psi": "/dev/psi/", "licences": "/licenses/",
}
TOP_LABELS = ["Guide", "Formulas", "Create Art", "Demo", "For developers", "Install"]
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
    want = {p for p in pages if not p.startswith("demo/") and p != "404.html" and p not in moved}
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
    for pat in ("/install/*", "/f/*", "/create/*", "/dev/psi/*", "/formulas/*"):
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
                if scheme == "light" and size == "laptop" and name in ("home", "install", "guide", "formulas", "create"):
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
                check(labels == TOP_LABELS and page.is_visible(".mnav nav a:has-text('Formulas')"), f"menu (phone): Menu opens the links {labels}")
                page.click(".mnav nav a:has-text('Formulas')")
                page.wait_for_url("**/formulas/")
                check(page.locator(".ftile").count() > 30, "menu (phone): Formulas opens the gallery")
            # The home page's way in.
            if scheme == "light":
                page.goto(base + "/")
                hrefs = page.eval_on_selector_all(".hero .cta", "els => els.map(e => e.getAttribute('href'))")
                doors = page.eval_on_selector_all(".door", "els => els.map(e => e.getAttribute('href'))")
                check(hrefs == ["install/", "demo/", "formulas/"] and doors == ["install/", "guide/", "formulas/"],
                      f"home ({size}): Install, the demo and Formulas up top {hrefs}; Install, the guide and the gallery below {doors}")
                lit_tiles = page.eval_on_selector_all(".tiles img, .hero img", "els => els.filter(i => i.getAttribute('src').startsWith('art/')).length")
                check(lit_tiles >= 8 and page.inner_text("h1").strip() != "", f"home ({size}): a heading, the panel and {lit_tiles} pieces of art")
                # The hero's Formula plays live once the page has loaded (home.js).
                wait_js(page, "+document.querySelector('.hero-panel [data-live=on] canvas')?.dataset.frames > 10")
                check(True, f"home ({size}): the hero plays its Formula live")
                if size == "phone":
                    hero = page.evaluate("document.querySelector('.hero-panel').getBoundingClientRect().top < document.querySelector('.hero h1').getBoundingClientRect().top")
                    check(hero, "home (phone): the panel comes first")
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


def api_pages(browser, site: Path, dist: Path, art: Path, errors: list[str]) -> None:
    page_html = site / "dev/api.html"
    check(page_html.is_file() and "HTTP API" in page_html.read_text(), "api: dev/api.html is in the site")
    carried = sorted(p.name for p in (site / "dev/api").glob("v*.html"))
    text = page_html.read_text() if page_html.is_file() else ""
    check(all(f'href="api/{n}"' in text for n in carried), f"api: dev/api.html links every frozen copy the site carries {carried}")
    ver = json.loads((dist / "manifest-4mb.json").read_text())["version"]
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


def psi_reference(browser, base: str, site: Path, errors: list[str], shots: Path) -> None:
    """/dev/psi/, the Psi language reference: in the menu (and linked from
    every page's), its generated tables and error index there, each code an
    anchor; the common errors' help (a before and an after), E407/E408
    marked internal; under its own CSP from _headers (the docs' policy and
    'wasm-unsafe-eval', set on the page's responses here too, so a violation
    shows without --base); nothing of Psi loaded until an example scrolls
    near; every live example (a cheat-sheet row's, a cookbook recipe's)
    drawn and lit once on screen, its Open in editor link opening the demo's
    editor with that formula; the search filtering rows and errors, the
    cheat sheet folding the page; no sideways scroll on a phone.
    Screenshots of it at both sizes in `shots`."""
    csp = csp_for(site, "/dev/psi/*")
    check("'wasm-unsafe-eval'" in csp and "connect-src" not in csp and "blob:" not in csp,
          "psi reference: its CSP is the docs' and 'wasm-unsafe-eval', nothing more")
    check((site / "dev/psi/index.html").is_file() and (site / "dev/psi/psiref.js").is_file(),
          "psi reference: dev/psi/index.html and its script are in the site")
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
        page.goto(f"{base}/dev/psi/#E202")
        page.wait_for_selector("h1")
        wait_js(page, "document.querySelector('main').dataset.psiRef == 'ready'")
        if size == "laptop":
            page.wait_for_timeout(600)
            early = [u for u in asked if "pkg-psi" in u]
            check(not early, f"psi reference: Psi doesn't load with the page, only when an example is near ({early[:1]})")
            check(page.title().startswith("Psi language reference"), f"psi reference: its title ({page.title()})")
            check(page.locator(".side a.on").inner_text() == "Psi language" and page.locator(".topnav a.on").inner_text() == "For developers",
                  "psi reference: in the developers' menu, marked")
            codes = page.eval_on_selector_all("h4[id^=E]", "els => els.map(e => e.id)")
            check(len(codes) >= 100 and "E202" in codes and "E601" in codes, f"psi reference: the error index ({len(codes)} codes, each an anchor)")
            top = page.evaluate("document.getElementById('E202').getBoundingClientRect().top")
            check(0 <= top < 400, f"psi reference: #E202 opens at its entry ({top:.0f} px from the top)")
            e202 = page.locator(".psi-err:has(#E202)").inner_text()
            check("Before:" in e202 and "After:" in e202 and "gray(sin(x))" in e202, "psi reference: E202 has a before and an after")
            internal = page.eval_on_selector_all("h4[id^=E]:has(.badge)", "els => els.map(e => e.id)")
            e407 = page.locator(".psi-err:has(#E407)").inner_text()
            e408 = page.locator(".psi-err:has(#E408)").inner_text()
            check(internal == ["E407"] and "a bug in Psi: please report it" in e407 and "not the formula" in e408,
                  f"psi reference: E407 marked internal, E408 explained ({internal})")
            cells = page.eval_on_selector_all("table code", "els => els.map(e => e.textContent)")
            check(all(any(n in c for c in cells) for n in ("atan2", "rainbow", "screen", "control")),
                  f"psi reference: the generated tables ({len(cells)} names)")
            check(page.locator("pre code", has_text="formula   =").count() == 1, "psi reference: the grammar")
            check(page.locator("pre code", has_text="psi check").count() >= 1, "psi reference: the psi command's example")
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
                lit_n = page.evaluate(f"""() => {{ const c = document.querySelectorAll('.psi-live canvas')[{k}];
                    const d = c.getContext('2d').getImageData(0, 0, 128, 32).data; let n = 0;
                    for (let i = 0; i < d.length; i += 4) if (d[i] + d[i+1] + d[i+2] > 90) n++; return n; }}""")
                if lit_n < 12:
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
                page.locator("#cookbook").scroll_into_view_if_needed()
                page.wait_for_timeout(500)
                page.screenshot(path=str(shots / "psi-reference-cookbook-laptop.png"))
                page.locator("#what-each-does").scroll_into_view_if_needed()
                page.wait_for_timeout(500)
                page.screenshot(path=str(shots / "psi-reference-examples-laptop.png"))
            # Search: rows and errors.
            page.fill("#psi-q", "cycle")
            page.wait_for_timeout(200)
            rows = page.eval_on_selector_all("table.psi-ex-t tbody tr:not([hidden])", "els => els.map(e => e.cells[0].textContent)")
            errs = page.eval_on_selector_all(".psi-err:not([hidden]) h4", "els => els.map(e => e.id)")
            shown_cook = page.locator("#cookbook").is_visible() or page.evaluate(
                "[...document.querySelectorAll('.psi-live.big, #cookbook ~ pre')].some(e => e.checkVisibility())")
            check(rows and all("cycle" in page.locator("table.psi-ex-t tbody tr:not([hidden])").nth(i).inner_text() for i in range(len(rows)))
                  and "cycle(fire, v, s)" in rows and "E931" in errs and "E202" not in errs and not shown_cook
                  and "built-in" in page.inner_text("#psi-n"),
                  f"psi reference: search 'cycle' leaves {len(rows)} rows and {len(errs)} errors, the rest hidden ({page.inner_text('#psi-n')})")
            if shots:
                page.evaluate("scrollTo(0, 0)")
                page.screenshot(path=str(shots / "psi-reference-search-laptop.png"))
            page.fill("#psi-q", "")
            page.click("#psi-cheat")
            page.wait_for_timeout(200)
            sheet = page.evaluate("""() => ({ cook: !!document.querySelector('#cookbook')?.checkVisibility(),
                rows: document.querySelectorAll('table.psi-ex-t tbody tr:not([hidden])').length,
                ex: [...document.querySelectorAll('.psi-live')].some(e => e.checkVisibility()),
                blocks: [...document.querySelectorAll('.psi-err blockquote')].some(e => e.checkVisibility()),
                titles: [...document.querySelectorAll('.psi-err h4')].filter(e => e.checkVisibility()).length })""")
            check(not sheet["cook"] and sheet["rows"] >= 60 and not sheet["ex"] and not sheet["blocks"] and sheet["titles"] >= 100,
                  f"psi reference: the cheat sheet keeps the rows and the error titles only ({sheet})")
            if shots:
                page.evaluate("scrollTo(0, 0)")
                page.screenshot(path=str(shots / "psi-reference-cheat-laptop.png"))
            page.click("#E931")
            page.wait_for_timeout(300)
            check(page.get_attribute("#psi-cheat", "aria-pressed") == "false" and page.locator(".psi-err:has(#E931) blockquote").is_visible()
                  and page.url.endswith("#E931"), "psi reference: an error's title in the cheat sheet opens its entry")
            # Open in editor: the site's editor with that formula (left in this site's storage).
            page.evaluate("scrollTo(0, 0)")
            first = page.locator(".psi-live a.psi-open").first
            first.scroll_into_view_if_needed()
            first.click()
            f_ready(page, 30000)
            stored = page.evaluate("localStorage.getItem('odmd.psi.open')")
            check(page.url == f"{base}/f/#open" and page.input_value("#psi-src").strip() != "" and stored is None,
                  f"psi reference: Open in editor opens the site's editor with it, read once ({page.input_value('#psi-src')[:40]!r})")
        else:
            page.wait_for_timeout(300)
            check(page.evaluate("document.documentElement.scrollWidth <= innerWidth"), "psi reference (phone): no sideways scroll")
            page.locator("#what-each-does").scroll_into_view_if_needed()
            wait_js(page, "[...document.querySelectorAll('.psi-live canvas')].some(c => c.dataset.frames > 0)", 15000)
            box = page.locator(".psi-live .led").first.bounding_box()
            check(page.evaluate("document.documentElement.scrollWidth <= innerWidth") and box and box["x"] + box["width"] <= 390,
                  "psi reference (phone): the examples fit, no sideways scroll")
            if shots:
                page.wait_for_timeout(400)
                page.screenshot(path=str(shots / "psi-reference-examples-phone.png"))
                page.locator("#cookbook").scroll_into_view_if_needed()
                page.wait_for_timeout(600)
                page.screenshot(path=str(shots / "psi-reference-cookbook-phone.png"))
        check(not violations, f"psi reference [{size}]: no CSP violations {violations[:2]}")
        page.context.close()


def psi_error_links(browser, base: str, errors: list[str]) -> None:
    """An error's code links the language reference's entry, on the site's
    own copy: /f/'s editor (a broken formula) and the demo's; the entry is
    there."""
    page = new_page(browser, "laptop", errors)
    open_text(page, base, "gray(sine(x))", "Broken")
    page.wait_for_selector("#psi-err a.ecode", timeout=20000)
    a = page.locator("#psi-err a.ecode")
    href = a.get_attribute("href")
    check(a.inner_text() == "E202" and href == f"{base}/dev/psi/#E202" and "Line 1" in page.inner_text("#psi-err"),
          f"/f/: a broken formula's code links the reference ({href})")
    page.goto(href)
    page.wait_for_selector("#E202")
    check(page.url == f"{base}/dev/psi/#E202", f"/f/: the code's link leads to its entry ({page.url})")
    page.goto(f"{base}/demo/#formulas")
    page.wait_for_selector("#psi-ex .psi-tile", timeout=30000)
    page.click("#psi-new")
    page.wait_for_selector("#psi-ed #psi-src", timeout=10000)
    page.fill("#psi-src", "gray(tme)")
    page.wait_for_selector("#psi-err a.ecode", timeout=10000)
    href = page.get_attribute("#psi-err a.ecode", "href")
    check(href == f"{base}/dev/psi/#E201", f"demo: the editor's error code links the site's own reference ({href})")
    page.context.close()


def install_links(site: Path) -> None:
    install = (site / "install/index.html").read_text()
    # One board: the DevKit's button. An 8 MB board in development (8mb) is hidden for now, but its
    # update bundle stays published for release.json's variants (the web app's update check).
    check(install.count("<esp-web-install-button") == 1 and 'manifest="firmware/manifest-4mb.json"' in install
          and "8mb" not in install and "8 MB build" not in install and "Not sure which" not in install,
          "install: one Install button, the DevKit's, and nothing about the hidden 8 MB board")
    fw = site / "install/firmware"
    rel_info = json.loads((site / "install/release.json").read_text())
    check((fw / "manifest-4mb.json").is_file() and (fw / "manifest.json").is_file() and not (fw / "manifest-8mb.json").exists(),
          "install: the 4mb manifest is published, the 8mb one is not")
    check(all((site / "install" / v["firmware"]).is_file() for v in rel_info["variants"].values()) and "8mb" in rel_info["variants"],
          "install: every release.json variant's bundle (8mb too) is published")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("site", type=Path)
    ap.add_argument("--shots", type=Path)
    ap.add_argument("--gallery-tool", type=Path, default=ROOT / "opendmd-gallery/target/release/opendmd-gallery")
    ap.add_argument("--dist", type=Path, default=ROOT / "dist", help="the release make site published (its manifest's version)")
    ap.add_argument("--art", type=Path, default=ROOT / "target/site-art", help="tools/site-art.py's output")
    ap.add_argument("--base", help="fetch the site from here (e.g. cf dev's http://127.0.0.1:8787) instead of serving SITE")
    a = ap.parse_args()
    (a.shots or ROOT / "target/site-smoke").mkdir(parents=True, exist_ok=True)
    base = a.base.rstrip("/") if a.base else serve(a.site)
    print(f"site: {base}{' (' + str(a.site) + ', served here)' if not a.base else ''}")
    errors: list[str] = []
    tool = a.gallery_tool if a.gallery_tool.is_file() else None
    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True)
        print("the site's structure (menu, home, themes, phone width, CSP, every page reachable, old addresses)")
        structure(browser, base, a.site, errors)
        print("the home page's hero show")
        hero_show(browser, base, errors)
        print("formula page (/f/)")
        f_page(browser, base, errors, tool, a.shots)
        print("Create Art (/create/)")
        create_page(browser, base, a.site, errors, a.shots)
        create_send(browser, base, a.site, errors)
        create_links(browser, base, a.site, errors)
        create_video(browser, base, a.site, errors, a.shots or ROOT / "target/site-smoke")
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
        api_pages(browser, a.site, a.dist, a.art, errors)
        print("the Psi language reference (dev/psi/)")
        psi_reference(browser, base, a.site, errors, a.shots)
        print("error codes linked to the reference (/f/, the demo)")
        psi_error_links(browser, base, errors)
        print("install page")
        install_links(a.site)
        if a.shots:
            screenshots(browser, base, errors, a.shots)
        browser.close()
    for e in errors:
        print(f"  FAIL {e}")
    failures.extend(errors)
    print(f"site-smoke: {'ok' if not failures else f'{len(failures)} failures'}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
