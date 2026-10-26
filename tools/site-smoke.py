# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright>=1.45", "pillow", "zxing-cpp>=2.2"]
# ///
"""Headless-Chromium smoke test of the site's formula pages, over a built
site (make site):

    uv run tools/site-smoke.py target/site [--shots DIR] [--gallery-tool PATH] [--dist dist --art target/site-art]

- /f/ (a formula's own page): the name and the code shown as text (a link's
  markup stays text), the preview lit and moving, a slider, the QR code
  decoding to the page's link, Copy link / Copy code, Try it in the demo
  (the demo's editor opens it), Open on my display (asked once: a bad
  address refused, then the display's `http://<address>/#formula=…`, then
  remembered), Submit to the gallery (GitHub's new-file address, whose file
  the gallery's own checker accepts; a too-long one falls back to copying),
  and a link with no formula;
- the demo's Copy share link gives a /f/ link that opens there;
- /art-index/v1/ (Create Art's game name index, not in the www pack): every
  platform's file published, the committed file byte for byte, served as
  JSON (the CORS header a player's web app needs is GitHub Pages' own,
  which this test's server cannot show); none left under demo/; the demo's
  Game art searches it same-origin (nothing asked of opendmd.github.io; the
  pictures' requests answered locally);
- /formulas/ (the gallery): every example listed with its thumbnail, Open
  and Remix leading to the formula;
- the HTTP API: dev/api.html in the site, linking every frozen copy it
  carries (dev/api/v<version>.html); then a publish-like build of the API
  pages (`site.py --api-only` into a copy of the site's dev/ and styles)
  with an archive standing in for the public repo's dev/api/ (an older
  release's frozen page, kept byte for byte; a stray file and a link, left
  out) and this release frozen: both exist and are listed newest first,
  the frozen page says which release it describes and links the latest;
  with `--api-freeze auto` off a tag, nothing new is frozen;
- /card/ ("Set up a card"), run with `?target=opfs` (the browser's private
  storage stands in for the picked card: the directory picker can't be
  driven headless) on the synthetic card zip tools/card-zip.py writes: the
  zip's counts; a run stopped half-way leaves only whole files and no
  playlists; an empty file and a swap file an interrupted write leaves, and
  a GIF of the same name but another size, are dealt with on the next run,
  which finishes the card: every GIF and playlist byte-identical to the
  zip, every lighter version byte-identical to `opendmd-pack convert`'s for
  the unzipped tree (`--pack`), cache/.failed with convert's lines, nothing
  the card doesn't need (the Windows program, the stray .mp4); then a run
  that writes nothing. Its visible text never names how the player keeps
  its lighter versions (CARD_LEAK); a browser without the directory picker
  gets the hand-copy sentence; a file that isn't a zip, or a zip that isn't
  a card, is refused; it asks nothing of any other site; the menu and the
  links to it.

- the demo's Settings, against its player in the page (js/demo.js): every
  tab; brightness, the colour, the Formula look, each GIF's stay, the
  playlist, night mode and a rule shown on its panel and in Now's banner;
  panel timing tried and kept, a new name, a key; firmware, WiFi and
  restarting shown with a line saying the demo can't; kept across a reload,
  not into a new session; every tab at laptop and phone sizes (in --shots,
  else target/site-smoke/).

Screenshots of /f/, the gallery and /card/ at laptop and phone sizes with --shots.
Needs, once, `uv run --with playwright playwright install chromium`.
"""

import argparse
import filecmp
import functools
import http.server
import io
import json
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import urllib.parse
import urllib.request
from pathlib import Path

import zxingcpp
from PIL import Image
from playwright.sync_api import Page, sync_playwright

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


def new_page(browser, size: str, errors: list[str]) -> Page:
    w, h = SIZES[size]
    phone = size == "phone"
    ctx = browser.new_context(viewport={"width": w, "height": h}, device_scale_factor=3 if phone else 1,
                              is_mobile=phone, has_touch=phone)
    ctx.grant_permissions(["clipboard-read", "clipboard-write"])
    page = ctx.new_page()
    # (The demo asks /api/status first: a static site answers 404, as on Pages.)
    page.on("console", lambda m: m.type == "error" and "/api/status" not in m.location.get("url", "")
            and errors.append(f"[{size}] console: {m.text} ({m.location.get('url')})"))
    page.on("pageerror", lambda e: errors.append(f"[{size}] pageerror: {e}"))
    page.on("response", lambda r: r.status >= 400 and "/demo/api/" not in r.url and "/api/status" not in r.url
            and errors.append(f"[{size}] HTTP {r.status} {r.url}"))
    return page


def lit(page: Page, sel: str) -> int:
    return page.evaluate("""(sel) => { const c = document.querySelector(sel); if (!c || !c.width) return 0;
        const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data; let n = 0;
        for (let i = 0; i < d.length; i += 4) if (d[i] + d[i+1] + d[i+2] > 90) n++; return n; }""", sel)


def share_link(base: str, src: str, name: str, **sliders) -> str:
    q = {"formula": src, "name": name, **{k: str(v) for k, v in sliders.items()}}
    return f"{base}/f/#" + urllib.parse.urlencode(q)


def params(u: str) -> dict:
    return urllib.parse.parse_qs(urllib.parse.urlsplit(u).fragment, keep_blank_values=True)


def f_page(browser, base: str, errors: list[str], tool: Path | None, shots: Path | None) -> None:
    src = "-- a: size\nloop 4\nd = len((x - 0.5)*4, y - 0.5)\npal(rainbow, frac(d*(1 + a) - t/2))"
    name = 'Rings <img src=x onerror="window.pwned=1">'
    url = share_link(base, src, name, a=0.25)
    for size in SIZES:
        page = new_page(browser, size, errors)
        page.goto(url)
        page.wait_for_selector("#f-name:not(:empty)", timeout=20000)
        page.wait_for_function("+(document.querySelector('#f-panel').dataset.frames || 0) > 5", timeout=20000)
        if size == "laptop":
            check(page.inner_text("#f-name") == name and page.evaluate("!window.pwned && !document.querySelector('#f-name img')"),
                  "/f/: the name is shown as text")
            check(page.locator("#f-code").inner_text().rstrip("\n") == src and page.locator("#f-code .n").count() > 3
                  and page.locator("#f-code .c").inner_text() == "-- a: size", "/f/: the code, highlighted")
            a0 = lit(page, "#f-panel")
            f0 = page.evaluate("+document.querySelector('#f-panel').dataset.frames")
            page.wait_for_timeout(400)
            check(a0 > 1000 and page.evaluate("+document.querySelector('#f-panel').dataset.frames") > f0,
                  f"/f/: the preview is lit ({a0} px) and moving")
            check(page.inner_text("#f-sl span") == "size" and page.input_value("#f-sl-a") == "0.25"
                  and "Loops every 4 s" in page.inner_text("#f-cap"), "/f/: a slider named by its comment, the caption")
            page.wait_for_timeout(400)
            # The QR code decodes to the page's link.
            png = page.locator("#f-qr svg").screenshot()
            img = Image.open(io.BytesIO(png))
            r = zxingcpp.read_barcodes(img)
            own = page.evaluate("location.href")
            check(bool(r) and r[0].text.split("#")[0] == own.split("#")[0] and params(r[0].text) == params(own), f"/f/: the QR code reads back the link ({r[0].text[:60] if r else None}…)")
            page.click("#f-copy-link")
            clip = page.evaluate("navigator.clipboard.readText()")
            check(clip.startswith(f"{base}/f/#formula=") and params(clip) == params(url), "/f/: Copy link copies the page's link")
            page.click("#f-copy-code")
            check(page.evaluate("navigator.clipboard.readText()") == src, "/f/: Copy code copies the formula")
            demo = page.get_attribute("#f-demo", "href")
            check(demo.startswith(f"{base}/demo/#formula=") and params(demo) == params(url), f"/f/: Try it in the demo -> {demo[:70]}…")
            # A slider moved: the links follow it.
            page.eval_on_selector("#f-sl-a", "r => { r.value = 0.5; r.dispatchEvent(new Event('input')); }")
            page.click("#f-copy-link")
            check(page.evaluate("navigator.clipboard.readText()").endswith("&a=0.5"), "/f/: the link carries the slider")
            # Open on my display: asked once, a bad address refused, then remembered.
            seen: list[str] = []
            page.context.route("http://192.168.1.50/**", lambda r: (seen.append(r.request.url), r.fulfill(body="<p>display</p>", content_type="text/html")))
            page.click("#f-open")
            check(page.is_visible("#f-ask") and page.input_value("#f-addr") == "opendmd.local", "/f/: Open on my display asks for the address (opendmd.local)")
            page.fill("#f-addr", "evil.example/x?")
            page.click("#f-go")
            check(page.is_visible("#f-addr-err") and not seen, "/f/: a bad address is refused")
            page.fill("#f-addr", "example.com")
            page.click("#f-go")
            check(page.is_visible("#f-addr-err") and "Press Open again" in page.inner_text("#f-addr-err") and not seen
                  and page.url.startswith(base), "/f/: an address on the internet is asked about first")
            page.fill("#f-addr", "http://192.168.1.50/")
            with page.expect_navigation():
                page.click("#f-go")
            want = {**params(url), "a": ["0.5"]}
            check(page.url.startswith("http://192.168.1.50/#formula=") and params(page.url) == want, f"/f/: it opens the display's editor: {page.url[:80]}…")
            page.go_back()
            page.wait_for_selector("#f-where:not([hidden])", timeout=20000)
            check(page.inner_text("#f-where-addr") == "192.168.1.50", "/f/: the address is remembered")
            with page.expect_navigation():
                page.click("#f-open")
            check(page.url.startswith("http://192.168.1.50/#formula="), "/f/: and used straight away the next time")
            page.go_back()
            page.wait_for_selector("#f-name:not(:empty)", timeout=20000)
            # Submit to the gallery: GitHub's new-file page with the file, which the checker accepts.
            page.click("#f-submit")
            check(page.is_visible("#f-sub") and page.input_value("#s-file") == "rings-img-src-x-onerror-window-pwned-1",
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
            check(text.startswith("-- name: Smoke rings\n-- description: Rings that ripple out\n-- author: @smoke\n-- licence: CC-BY-4.0\n-- values: a=0.25\n")
                  and text.endswith(src + "\n") and shown == text, f"/f/: the file has the gallery's header {text!r}")
            if tool:
                with tempfile.TemporaryDirectory() as d:
                    p = Path(d) / "smoke-rings.psi"
                    p.write_text(text)
                    r = subprocess.run([str(tool), "check", str(p)], capture_output=True, text=True)
                    check(r.returncode == 0, f"/f/: the gallery's checker accepts it: {r.stderr.strip()}")
            if shots:
                page.locator("#f-sub").scroll_into_view_if_needed()
                page.screenshot(path=str(shots / "f-submit-laptop.png"))
        else:
            check(page.evaluate("document.documentElement.scrollWidth <= innerWidth"), "/f/ (phone): no sideways scroll")
        page.context.close()

    # A long formula: GitHub can't take it in an address, so the file is copied.
    page = new_page(browser, "laptop", errors)
    long_src = "loop 4\n" + "".join(f"-- {',' * 60}\n" for k in range(50)) + "gray(x)"
    page.goto(share_link(base, long_src, "Long one") + "&submit")
    page.wait_for_selector("#f-sub:not([hidden])", timeout=20000)
    page.fill("#s-desc", "Long")
    page.fill("#s-author", "me")
    href = page.get_attribute("#s-go", "href")
    check(page.is_visible("#s-long") and href.endswith("/new/main/gallery?filename=long-one.psi"),
          f"/f/: a formula too long for GitHub's address: copy the file instead ({href[:90]}, {page.inner_text('#s-err')})")
    page.click("#s-copy")
    check(page.evaluate("navigator.clipboard.readText()").endswith(long_src + "\n"), "/f/: Copy the file")
    # No formula in the link.
    page.goto(f"{base}/f/")
    page.wait_for_timeout(500)
    check(page.is_visible("#f-none") and page.is_hidden("#f"), "/f/: a link with no formula says so")
    # Try it in the demo: the demo's editor has it.
    page.goto("about:blank")
    page.goto(url)
    page.wait_for_selector("#f-name:not(:empty)", timeout=20000)
    page.click("#f-demo")
    page.wait_for_selector("#psi-ed #psi-src", timeout=30000)
    page.wait_for_timeout(500)
    check(page.input_value("#psi-src") == src and page.input_value("#psi-name") == name[:60],
          "/f/ -> the demo: the editor opens the formula")
    page.context.close()


def demo_share(browser, base: str, errors: list[str]) -> None:
    """The demo's Copy share link gives a /f/ link."""
    page = new_page(browser, "laptop", errors)
    page.goto(f"{base}/demo/#formulas")
    page.wait_for_selector("#psi-ex .psi-tile", timeout=30000)
    page.click("#psi-ex .psi-tile >> nth=0")
    page.click("#psi-share-btn")
    link = page.input_value("#psi-link")
    sub = page.get_attribute("#psi-submit", "href")
    check(link.startswith(f"{base}/f/#formula=") and sub == link + "&submit", f"demo: Copy share link -> {link[:60]}…")
    page.goto(link)
    page.wait_for_selector("#f-name:not(:empty)", timeout=20000)
    first = json.loads((ROOT / "opendmd-web/www/formulas.json").read_text())[0]
    check(page.inner_text("#f-name") == first["name"], f"demo: the link opens {first['name']} on its page")
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
        page.wait_for_function(f"document.querySelector({json.dumps(sel)}).closest('.card,.grp').querySelector('.tick')?.textContent.includes('Saved')", timeout=10000)

    page = new_page(browser, "laptop", errors)
    page.goto(f"{base}/demo/#now")
    page.wait_for_function("(t => t && t !== 'Local simulator')(document.querySelector('#now-title')?.textContent)", timeout=30000)
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
    page.wait_for_function("[...document.querySelectorAll('#st-look-formula option')].some(o => o.textContent === 'Scanlines')", timeout=20000)
    page.select_option("#st-look-formula", label="Scanlines")
    saved("#st-look-formula")
    now()
    page.wait_for_timeout(800)
    after = rows_ratio(page)
    check(abs(after / before - 1) > 0.2 and page.evaluate("import('./js/state.js').then(m => m.app.status.formula_look?.error ?? null)") is None,
          f"demo settings: the Scanlines look is drawn over the panel (even/odd rows {before:.2f} -> {after:.2f})")
    # Any Formula as the look: Plasma covers the GIF, so it starts at Screen 40%.
    lum = lambda: page.evaluate("""() => { const c = document.querySelector('#now-panel'), d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
        let n = 0; for (let i = 0; i < d.length; i += 4) n += d[i] + d[i+1] + d[i+2]; return n; }""")
    l0 = lum()
    tab("look")
    page.evaluate("document.querySelectorAll('#st-tab-look .tick').forEach(t => t.textContent = '')")
    page.select_option("#st-look-formula", label="Plasma")
    saved("#st-look-formula")
    mix = (page.input_value("#st-look-blend"), page.input_value("#st-look-amt"))
    cfg = page.evaluate("import('./js/state.js').then(m => [m.app.config.look_formula_name, m.app.status.formula_look?.error ?? null])")
    now()
    page.wait_for_timeout(800)
    l1, ratio = lum(), rows_ratio(page)
    check(mix == ("screen", "40") and cfg == ["Plasma · Screen 40%", None] and l1 > 1.2 * l0 and abs(ratio - 1) < 0.15,
          f"demo settings: Plasma screened at 40% over the panel ({mix}, {cfg}; brightness {l0:,} -> {l1:,}, even/odd rows {ratio:.2f})")
    page.screenshot(path=str(shots / "demo-look-plasma-laptop.png"))
    tab("look")
    page.click("#st-trans-preview")
    page.wait_for_timeout(1500)
    check(lit(page, "#st-trans-panel") > 500, "demo settings: the transition preview plays")
    # The schedule: night mode from a minute ago for an hour, Panel off.
    tab("time")
    page.wait_for_function("/^\\d\\d:\\d\\d/.test(document.querySelector('#tm-now').textContent)", timeout=10000)
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
    page.wait_for_function("document.querySelector('#now-banner').textContent.startsWith('Schedule paused')", timeout=10000)
    check("brightness(0.5" in filt(), f"demo settings: Wake for an hour lights it again ({page.inner_text('#now-banner')}, {filt()})")
    page.click("#now-resume")
    page.wait_for_function("document.querySelector('#now-banner').textContent.startsWith('Night mode')", timeout=10000)
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
    page.wait_for_function("document.querySelector('#fo-result').textContent.includes('demo')", timeout=10000)
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
    page.wait_for_function("document.querySelector('#st-moved').textContent.includes('kitchen.local')", timeout=15000)
    page.fill("#st-ssid", "Elsewhere")
    page.click("#st-save-net")
    page.wait_for_function("document.querySelector('#st-net .tick').textContent.includes('demo')", timeout=10000)
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
    page.wait_for_function("(t => t && t !== 'Local simulator')(document.querySelector('#now-title')?.textContent)", timeout=30000)
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
        page.wait_for_function("!document.querySelector('#where').textContent.startsWith('Demo')", timeout=10000)
        page.evaluate("location.hash = '#settings'")
        page.wait_for_selector("#view-settings.on .card", timeout=10000)
        page.evaluate("location.hash = '#now'")
        page.wait_for_selector("#view-now.on #sim-play", timeout=10000)
        page.select_option("#sim-list", "/filelist.txt")
        page.click("#sim-play")
        page.wait_for_function(f"document.querySelector('#now-title').textContent !== 'Local simulator'", timeout=10000)
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


def art_index(browser, base: str, site: Path, errors: list[str]) -> None:
    """The game name index at art-index/v1/, and the demo using it same-origin."""
    bad = []
    cors = None
    for plat in ART_PLATFORMS:
        f = site / "art-index/v1" / f"{plat}.json"
        src = ROOT / "opendmd-web/art-index" / f"{plat}.json"
        if not f.is_file() or f.read_bytes() != src.read_bytes():
            bad.append(plat)
            continue
        req = urllib.request.Request(f"{base}/art-index/v1/{plat}.json", headers={"Origin": "http://192.168.1.50"})
        with urllib.request.urlopen(req) as r:
            body, ctype, cors = r.read(), r.headers.get("content-type", ""), r.headers.get("access-control-allow-origin")
        d = json.loads(body)
        if r.status != 200 or "json" not in ctype or len(d["names"]) != len(d["kinds"]) or not d["names"]:
            bad.append(plat)
    check(not bad, f"art index: art-index/v1/<platform>.json for all {len(ART_PLATFORMS)} platforms, fetchable as JSON "
                   f"(CORS header here: {cors or 'none, the test server; Pages sends *'}){' — bad: ' + ', '.join(bad) if bad else ''}")
    check(not any(p.is_file() for p in (site / "demo/art").rglob("*")), "art index: not under demo/ (the www pack does not carry it)")
    png = io.BytesIO()
    Image.new("RGB", (32, 24), (200, 40, 40)).save(png, "PNG")
    page = new_page(browser, "laptop", errors)
    asked: list[str] = []
    site_asked: list[str] = []
    page.route("https://raw.githubusercontent.com/**", lambda r: r.fulfill(
        status=200, body=png.getvalue(), headers={"access-control-allow-origin": "*", "content-type": "image/png"}))
    page.route("https://opendmd.github.io/**", lambda r: (site_asked.append(r.request.url), r.abort()))
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
          f"asked of opendmd.github.io: {site_asked})")
    page.context.close()


def gallery(browser, base: str, errors: list[str], shots: Path | None) -> None:
    examples = json.loads((ROOT / "opendmd-web/www/formulas.json").read_text())
    for size in SIZES:
        page = new_page(browser, size, errors)
        page.goto(f"{base}/formulas/")
        page.wait_for_selector(".ftile", timeout=20000)
        page.wait_for_function("[...document.querySelectorAll('.ftile img')].slice(0, 4).every(i => i.complete && i.naturalWidth == 128)",
                               timeout=20000)
        names = page.eval_on_selector_all("#examples ~ ul .ftile h3", "els => els.map(e => e.textContent)")
        if size == "laptop":
            check(names == [e["name"] for e in examples], f"gallery: the {len(names)} examples, in order")
            check(page.locator("nav a.on").inner_text() == "Formula gallery", "gallery: in the menu")
            community = page.eval_on_selector_all("#community + ul .ftile h3", "els => els.map(e => e.textContent)")
            print(f"  ..   gallery: {len(community)} from the community {community[:5]}")
            page.evaluate("document.querySelectorAll('.ftile img').forEach(i => i.loading = 'eager')")
            page.wait_for_function("[...document.querySelectorAll('.ftile img')].every(i => i.complete && i.naturalWidth == 128)", timeout=20000)
            dark = page.evaluate("""() => [...document.querySelectorAll('.ftile img')].filter(i => {
                const c = document.createElement('canvas'); c.width = 128; c.height = 32; const x = c.getContext('2d'); x.drawImage(i, 0, 0);
                const d = x.getImageData(0, 0, 128, 32).data; for (let k = 0; k < d.length; k += 4) if (d[k] + d[k+1] + d[k+2] > 90) return false; return true; }).length""")
            check(dark == 0, f"gallery: every thumbnail lit ({dark} dark)")
            remix = page.locator("#examples ~ ul .ftile").nth(0).locator(".facts a.alt").get_attribute("href")
            check(remix.startswith("../demo/#formula=") and "remix" in urllib.parse.unquote_plus(remix),
                  "gallery: Remix opens it in the demo")
            page.locator("#examples ~ ul .ftile").nth(5).locator(".facts a:not(.alt)").click()
            page.wait_for_selector("#f-name:not(:empty)", timeout=20000)
            e = examples[5]
            check(page.inner_text("#f-name") == e["name"] and page.locator("#f-code").inner_text().rstrip("\n") == e["src"],
                  f"gallery: Open leads to {e['name']}'s page")
            # A Formula that draws over a GIF: /f/ shows it over the sample picture, not black.
            e = next(x for x in examples if x["name"] == "Scanlines")
            page.goto(share_link(base, e["src"], e["name"], a=e["a"]))
            page.wait_for_function("+(document.querySelector('#f-panel').dataset.frames || 0) > 5", timeout=20000)
            n = lit(page, "#f-panel")
            check(n > 1000, f"/f/: {e['name']} (over a GIF) is lit, over the sample picture ({n} px)")
        else:
            check(page.evaluate("document.documentElement.scrollWidth <= innerWidth"), "gallery (phone): no sideways scroll")
        page.context.close()


def screenshots(browser, base: str, errors: list[str], shots: Path) -> None:
    """/f/ (an example, as the gallery's Open gives it) and the gallery, at
    both sizes: the viewport and the whole page."""
    e = next(x for x in json.loads((ROOT / "opendmd-web/www/formulas.json").read_text()) if x["name"] == "Stripes")  # a slider
    for size in SIZES:
        page = new_page(browser, size, errors)
        page.goto(share_link(base, e["src"], e["name"], a=e.get("a", 0)))
        page.wait_for_function("+(document.querySelector('#f-panel').dataset.frames || 0) > 20", timeout=20000)
        page.wait_for_timeout(600)
        page.screenshot(path=str(shots / f"f-{size}.png"))
        page.screenshot(path=str(shots / f"f-{size}-full.png"), full_page=True)
        page.goto(f"{base}/formulas/")
        page.evaluate("document.querySelectorAll('.ftile img').forEach(i => i.loading = 'eager')")
        page.wait_for_function("[...document.querySelectorAll('.ftile img')].every(i => i.complete && i.naturalWidth == 128)", timeout=20000)
        page.screenshot(path=str(shots / f"gallery-{size}.png"))
        page.screenshot(path=str(shots / f"gallery-{size}-full.png"), full_page=True)
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
        check(f"OpenDMD {ver}" in page.inner_text("p.vers") and page.locator("h2").count() > 10,
              f"api: dev/api.html's link opens the frozen page ({page.locator('h2').count()} sections)")
        page.click("p.vers a")
        page.wait_for_url("**/dev/api.html")
        check(page.locator('nav a.on[href="api.html"]').count() == 1, "api: the frozen page's link leads back to the latest")
        page.context.close()
        # Off a tag (`git describe` is not v<version>), auto freezes nothing.
        desc = subprocess.run(["git", "describe", "--always", "--dirty", "--tags", "--match", "v*"], cwd=ROOT, capture_output=True, text=True).stdout.strip()
        if desc != f"v{ver}":
            build("auto")
            names = sorted(p.name for p in (out / "dev/api").iterdir())
            check(names == ["v0.0.1.html"], f"api: off the tag ({desc}), --api-freeze auto freezes nothing new {names}")


# Words "Set up a card" never shows: how the player keeps a GIF's lighter version is its business.
CARD_LEAK = re.compile(r"odm|bucket|cache|prepar|cop(y|ies) of|lighter (version|cop)|\bcopies\b|\.crswap|opfs", re.I)

# The card in the page's private storage, every file as base64 (and its
# lastModified), and helpers to clear it and to put files in it.
OPFS_JS = """
async () => {
  const root = await navigator.storage.getDirectory();
  const out = {};
  async function walk(d, pre) {
    for await (const [name, h] of d.entries()) {
      if (h.kind === 'directory') { await walk(h, pre + name + '/'); continue; }
      const f = await h.getFile();
      const b = new Uint8Array(await f.arrayBuffer());
      let s = '';
      for (let i = 0; i < b.length; i += 0x8000) s += String.fromCharCode(...b.subarray(i, i + 0x8000));
      out[pre + name] = [btoa(s), f.lastModified];
    }
  }
  await walk(root, '');
  return out;
}"""
OPFS_CLEAR = """async () => { const r = await navigator.storage.getDirectory();
  for await (const [n] of r.entries()) await r.removeEntry(n, { recursive: true }); }"""
OPFS_PUT = """async ([path, b64]) => {
  const parts = path.split('/'); const name = parts.pop();
  let d = await navigator.storage.getDirectory();
  for (const p of parts) d = await d.getDirectoryHandle(p, { create: true });
  const w = await (await d.getFileHandle(name, { create: true })).createWritable();
  await w.write(Uint8Array.from(atob(b64), (c) => c.charCodeAt(0))); await w.close(); }"""


def card_page(browser, base: str, errors: list[str], pack: Path, shots: Path | None) -> None:
    import base64
    import zipfile
    with tempfile.TemporaryDirectory() as t:
        tmp = Path(t)
        r = subprocess.run(["uv", "run", "--quiet", str(ROOT / "tools/card-zip.py"), str(tmp / "fx")], capture_output=True, text=True)
        check(r.returncode == 0, f"card: the synthetic card zip (tools/card-zip.py) {r.stderr.strip()[-200:]}")
        zpath = tmp / "fx/card.zip"
        conv = tmp / "conv"
        shutil.copytree(tmp / "fx/tree", conv)
        r = subprocess.run([str(pack), "convert", str(conv)], capture_output=True, text=True)
        check(r.returncode == 0, f"card: opendmd-pack convert on the unzipped tree {r.stderr.strip()[-200:]}")
        want_copies = {p.relative_to(conv).as_posix(): p.read_bytes() for p in (conv / "cache").rglob("*") if p.is_file() and p.name != ".failed"}
        want_failed = (conv / "cache/.failed").read_text()
        with zipfile.ZipFile(zpath) as z:
            zfiles = {i.filename: z.read(i) for i in z.infolist() if not i.is_dir()}
        on_card = {k: v for k, v in zfiles.items() if (k.startswith("gifs/") and k.lower().endswith(".gif"))
                   or k in ("filelist.txt", "filelist.hall", "filelist.xmas", "logos/boot.gif")}
        gifs = [k for k in on_card if k.startswith("gifs/")]
        cats = {k.split("/")[1] for k in gifs if k.count("/") > 1}
        requests: list[str] = []

        def visible(page: Page, where: str) -> None:
            text = page.inner_text("main")
            hits = sorted({m.group(0) for m in CARD_LEAK.finditer(text)})
            check(not hits, f"card: no hidden-machinery words on the page ({where}) {hits}")

        def card(page: Page) -> dict[str, tuple[bytes, int]]:
            return {k: (base64.b64decode(v[0]), v[1]) for k, v in page.evaluate(OPFS_JS).items()}

        def open_page(query: str, size: str = "laptop", page: Page | None = None) -> Page:
            """/card/ in a new context, or again in `page`'s (the same private storage: the same card)."""
            if page is None:
                page = new_page(browser, size, errors)
                page.on("request", lambda rq: requests.append(rq.url))
            page.goto(f"{base}/card/{query}")
            page.wait_for_selector("#cp[data-state=idle]", timeout=20000)
            return page

        def take_zip(page: Page) -> None:
            page.set_input_files("#cp-file", str(zpath))
            page.wait_for_selector("#cp[data-state=read]", timeout=20000)

        def pick(page: Page) -> None:
            page.click("#cp-write")
            page.wait_for_selector("#cp[data-state=listed], #cp[data-state=ready]", timeout=20000)

        # 1. The zip, and a run stopped half-way.
        page = open_page("?target=opfs&pace=500")
        visible(page, "on arrival")
        check(page.locator("nav a.on").inner_text() == "Set up a card"
              and page.evaluate("[...document.querySelectorAll('nav a')].map(a => a.textContent).join('|')").count("Install|Set up a card|") == 1,
              "card: in the menu, right after Install")
        check(page.is_disabled("#cp-write") and page.is_hidden("#cp-nowrite"), "card: Write to the card… waits for the zip")
        page.evaluate(OPFS_CLEAR)
        take_zip(page)
        found = page.inner_text("#cp-found-text")
        check(found.startswith(f"Found {len(gifs)} GIFs in {len(cats)} categories and 3 playlists (All files, Halloween, Christmas)"),
              f"card: what the zip holds: {found}")
        visible(page, "the zip read")
        pick(page)
        listed = page.inner_text("#cp-card-text")
        check(listed.startswith(f"{len(gifs)} GIFs to write"), f"card: an empty card: {listed}")
        page.click("#cp-start")
        page.wait_for_function("+document.querySelector('#cp').dataset.done >= 2", timeout=30000)
        status = page.inner_text("#cp-status")
        check(status.startswith("Writing GIFs and getting them ready · ") and f" of {len(gifs)}" in status, f"card: the progress line: {status}")
        page.click("#cp-stop")
        page.wait_for_selector("#cp[data-state=stopped]", timeout=30000)
        done = int(page.get_attribute("#cp", "data-done"))
        damaged_before = int(page.get_attribute("#cp", "data-damaged"))
        check(page.inner_text("#cp-done-text").startswith(f"Stopped after {done} of {len(gifs)} GIFs"),
              f"card: stopped: {page.inner_text('#cp-done-text')}")
        got = card(page)
        partial = [k for k, (b, _) in got.items() if (k in on_card and b != on_card[k]) or (k.startswith("cache/") and k != "cache/.failed" and b != want_copies.get(k))]
        check(0 < done < len(gifs) and not partial and not any(k.endswith(".crswap") for k in got),
              f"card: a stopped run leaves only whole files ({len(got)} files after {done} GIFs; bad: {partial})")
        check(not any(k.startswith("filelist") for k in got), "card: and no playlists: they go last")
        visible(page, "stopped")

        # 2. What an interrupted write leaves, and another file of the same name; then the run that finishes the card.
        written = sorted(k for k in got if k.startswith("gifs/"))
        unwritten = sorted(k for k in gifs if k not in got)
        page = open_page("?target=opfs", page=page)
        empty, swap = unwritten[0], unwritten[1] + ".crswap"
        page.evaluate(OPFS_PUT, [empty, ""])
        page.evaluate(OPFS_PUT, [swap, base64.b64encode(b"half a swap file").decode()])
        other = written[0]
        page.evaluate(OPFS_PUT, [other, base64.b64encode(b"GIF89a another file of the same name").decode()])
        other_copy = next((k for k in got if k.startswith("cache/") and k.endswith("/" + other.split("/")[-1])), None)
        if other_copy:
            page.evaluate(OPFS_PUT, [other_copy, base64.b64encode(b"GIF89a its old lighter version").decode()])
        take_zip(page)
        pick(page)
        listed = page.inner_text("#cp-card-text")
        check(listed.startswith("Picking up where it stopped: ") and page.get_attribute("#cp", "data-swaps") == "1",
              f"card: half a card: {listed}")
        page.click("#cp-start")
        page.wait_for_selector("#cp[data-state=done]", timeout=60000)
        summary = page.inner_text("#cp-done")
        check(summary.startswith(f"Your card is ready: {len(gifs)} GIFs and 3 playlists (All files, Halloween, Christmas), "
                                 "ready to play from the first boot."), f"card: done: {summary[:160]}")
        # (Those met before the stop are in cache/.failed, but not as "plays as it is": tried again, met again.)
        damaged = int(page.get_attribute("#cp", "data-damaged"))
        check(damaged == 2 and "2 GIFs can't be played by anything" in summary
              and "1 line of the playlists names GIFs the pack doesn't have" in summary and "Windows program" in summary,
              f"card: the notes: unplayable GIFs ({damaged_before} met before the stop, {damaged} in the finishing run), "
              "the missing line, the Windows program")
        check(page.is_visible("#cp-eject") and "Eject the card before you take it out" in page.inner_text("#cp-eject"),
              "card: it says to eject the card first")
        visible(page, "done")
        got = card(page)
        cards = {k: b for k, (b, _) in got.items()}
        check(all(cards.get(k) == v for k, v in on_card.items()), "card: every GIF, the playlists and the boot logo byte-identical to the zip")
        mine = {k: v for k, v in cards.items() if k.startswith("cache/") and k != "cache/.failed"}
        check(mine == want_copies, f"card: every lighter version byte-identical to opendmd-pack convert's ({len(mine)} of {len(want_copies)};"
              f" differing: {sorted(k for k in set(mine) | set(want_copies) if mine.get(k) != want_copies.get(k))[:5]})")
        failed = cards.get("cache/.failed", b"").decode()
        asis = [l for l in failed.splitlines(keepends=True) if l.endswith("plays as it is\n")]
        rest = sorted(l.split("\t")[0] for l in failed.splitlines() if not l.endswith("plays as it is"))
        check("".join(asis) == want_failed and rest == ["/gifs/Pinball_Short/PINBALL_Cut_Short.gif", "/gifs/Pinball_Short/PINBALL_Not_A_GIF.gif"],
              f"card: cache/.failed: convert's lines, and the two nothing can play {failed!r}")
        extra = sorted(set(cards) - set(on_card) - set(want_copies) - {"cache/.failed"})
        check(not extra, f"card: nothing else on the card (no Windows program, no .mp4, no swap file) {extra}")

        # 3. A finished card: nothing to write. The two GIFs nothing decodes are
        # offered again (cache/.failed lines other than "plays as it is" are
        # retried), and trying them writes nothing.
        page = open_page("?target=opfs", page=page)
        take_zip(page)
        pick(page)
        check(page.get_attribute("#cp", "data-state") == "listed" and page.get_attribute("#cp", "data-todo") == "2"
              and page.inner_text("#cp-card-text") == "Everything is already on this card: it's ready to play. "
              "2 GIFs couldn't be got ready before; Start tries them again.",
              f"card: a finished card: {page.inner_text('#cp-card-text')}")
        page.click("#cp-start")
        page.wait_for_selector("#cp[data-state=done]", timeout=30000)
        check(card(page) == got and page.get_attribute("#cp", "data-written") == "0" and page.get_attribute("#cp", "data-made") == "0",
              "card: trying them again writes nothing, and nothing on the card is touched")
        if shots:
            page.screenshot(path=str(shots / "card-laptop-ready.png"), full_page=True)
        page.evaluate(OPFS_CLEAR)

        # 4. The zip changed on disk mid-run (moved, deleted, re-downloaded): the run stops, says so, writes no playlists.
        moving = tmp / "moving.zip"
        shutil.copyfile(zpath, moving)
        page = open_page("?target=opfs&pace=500", page=page)
        page.set_input_files("#cp-file", str(moving))
        page.wait_for_selector("#cp[data-state=read]", timeout=20000)
        pick(page)
        page.click("#cp-start")
        page.wait_for_function("+document.querySelector('#cp').dataset.done >= 2", timeout=30000)
        moving.write_bytes(b"gone")
        page.wait_for_selector("#cp[data-state=failed]", timeout=60000)
        gone = page.inner_text("#cp-done-text")
        names = set(card(page))
        check(gone.startswith("The zip couldn't be read any more after") and "moved" in gone
              and not any(k.startswith("filelist") for k in names) and int(page.get_attribute("#cp", "data-failed")) == 0,
              f"card: the zip gone mid-run stops the run, no playlists: {gone}")
        page.evaluate(OPFS_CLEAR)
        page.context.close()

        # 5. Not a zip; a zip that isn't a card.
        page = open_page("")
        notzip = tmp / "pack.zip"
        notzip.write_bytes(b"GIF89a this is not a zip " * 40)
        page.set_input_files("#cp-file", str(notzip))
        page.wait_for_selector("#cp[data-state=error]", timeout=20000)
        check("can't be read as a zip" in page.inner_text("#cp-zip-err") and page.is_disabled("#cp-write"), "card: a file that isn't a zip is refused")
        other_zip = tmp / "photos.zip"
        with zipfile.ZipFile(other_zip, "w") as z:
            z.writestr("holiday/beach.jpg", b"\xff\xd8 not really")
        page.set_input_files("#cp-file", str(other_zip))
        page.wait_for_function("document.querySelector('#cp-zip-err').textContent.includes('look like a card')", timeout=20000)
        check(page.is_visible("#cp-zip-err"), f"card: a zip that isn't a card is refused: {page.inner_text('#cp-zip-err')}")
        page.context.close()

        # 6. A browser that can't write to a folder.
        ctx_page = new_page(browser, "laptop", errors)
        ctx_page.add_init_script("delete window.showDirectoryPicker")
        ctx_page.goto(f"{base}/card/")
        ctx_page.wait_for_selector("#cp[data-state=idle]", timeout=20000)
        check(ctx_page.is_visible("#cp-nowrite") and ctx_page.is_hidden("#cp-write")
              and "Open this page in Chrome or Edge on a computer" in ctx_page.inner_text("#cp-nowrite"),
              "card: without the directory picker, the hand-copy sentence instead of the button")
        ctx_page.context.close()

        # 7. Phone width; and the links to the page.
        page = open_page("", "phone")
        check(page.evaluate("document.documentElement.scrollWidth <= innerWidth"), "card (phone): no sideways scroll")
        if shots:
            page.screenshot(path=str(shots / "card-phone.png"), full_page=True)
        page.context.close()
        if shots:
            page = open_page("?target=opfs")
            take_zip(page)
            page.screenshot(path=str(shots / "card-laptop.png"), full_page=True)
            page.context.close()
        foreign = sorted({u for u in requests if not u.startswith(base)})
        check(not foreign, f"card: no request to any other site {foreign}")


def card_links(site: Path) -> None:
    install = (site / "install/index.html").read_text()
    guide = (site / "guide/card.html").read_text()
    page = (site / "card/index.html").read_text()
    licences = (site / "licenses/index.html").read_text()
    check('href="../card/index.html">Set up a card</a> from the' in install, "card: the install page's last step links it")
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
    check('href="../card/index.html">Set up a card</a> write it' in guide, "card: the SD card guide links it")
    check('href="https://dmdos.net/">dmdos.net</a>' in page and "download.php" not in page and ".zip\"" not in page,
          "card: the pack is linked at dmdos.net's site, not the file")
    check("Set up a card</strong> runs the web app's WebAssembly in your browser" in licences, "card: the licences page says what it runs")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("site", type=Path)
    ap.add_argument("--shots", type=Path)
    ap.add_argument("--gallery-tool", type=Path, default=ROOT / "opendmd-gallery/target/release/opendmd-gallery")
    ap.add_argument("--dist", type=Path, default=ROOT / "dist", help="the release make site published (its manifest's version)")
    ap.add_argument("--art", type=Path, default=ROOT / "target/site-art", help="tools/site-art.py's output")
    ap.add_argument("--pack", type=Path, default=ROOT / "opendmd-pack/target/release/opendmd-pack",
                    help="opendmd-pack, built (the reference for /card/'s lighter versions)")
    a = ap.parse_args()
    (a.shots or ROOT / "target/site-smoke").mkdir(parents=True, exist_ok=True)
    base = serve(a.site)
    errors: list[str] = []
    tool = a.gallery_tool if a.gallery_tool.is_file() else None
    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True)
        print("formula page (/f/)")
        f_page(browser, base, errors, tool, a.shots)
        print("the demo's Settings")
        demo_settings(browser, base, errors, a.shots or ROOT / "target/site-smoke")
        demo_own_card(browser, base, a.site, errors)
        print("the demo's share link")
        demo_share(browser, base, errors)
        print("the art index (/art-index/)")
        art_index(browser, base, a.site, errors)
        print("the gallery (/formulas/)")
        gallery(browser, base, errors, a.shots)
        print("the HTTP API (dev/api.html, dev/api/)")
        api_pages(browser, a.site, a.dist, a.art, errors)
        print("set up a card (/card/)")
        card_links(a.site)
        card_page(browser, base, errors, a.pack, a.shots)
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
