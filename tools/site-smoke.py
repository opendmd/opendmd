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
- /formulas/ (the gallery): every example listed with its thumbnail, Open
  and Remix leading to the formula;
- the HTTP API: dev/api.html in the site, linking every frozen copy it
  carries (dev/api/v<version>.html); then a publish-like build of the API
  pages (`site.py --api-only` into a copy of the site's dev/ and styles)
  with an archive standing in for the public repo's dev/api/ (an older
  release's frozen page, kept byte for byte; a stray file and a link, left
  out) and this release frozen: both exist and are listed newest first,
  the frozen page says which release it describes and links the latest;
  with `--api-freeze auto` off a tag, nothing new is frozen.

Screenshots of /f/ and the gallery at laptop and phone sizes with --shots.
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
        else:
            check(page.evaluate("document.documentElement.scrollWidth <= innerWidth"), "gallery (phone): no sideways scroll")
        page.context.close()


def screenshots(browser, base: str, errors: list[str], shots: Path) -> None:
    """/f/ (an example, as the gallery's Open gives it) and the gallery, at
    both sizes: the viewport and the whole page."""
    e = json.loads((ROOT / "opendmd-web/www/formulas.json").read_text())[8]  # Stripes: a slider
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
    ver = json.loads((dist / "manifest.json").read_text())["version"]
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
        desc = subprocess.run(["git", "describe", "--always", "--dirty", "--tags"], cwd=ROOT, capture_output=True, text=True).stdout.strip()
        if desc != f"v{ver}":
            build("auto")
            names = sorted(p.name for p in (out / "dev/api").iterdir())
            check(names == ["v0.0.1.html"], f"api: off the tag ({desc}), --api-freeze auto freezes nothing new {names}")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("site", type=Path)
    ap.add_argument("--shots", type=Path)
    ap.add_argument("--gallery-tool", type=Path, default=ROOT / "opendmd-gallery/target/release/opendmd-gallery")
    ap.add_argument("--dist", type=Path, default=ROOT / "dist", help="the release make site published (its manifest's version)")
    ap.add_argument("--art", type=Path, default=ROOT / "target/site-art", help="tools/site-art.py's output")
    a = ap.parse_args()
    if a.shots:
        a.shots.mkdir(parents=True, exist_ok=True)
    base = serve(a.site)
    errors: list[str] = []
    tool = a.gallery_tool if a.gallery_tool.is_file() else None
    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True)
        print("formula page (/f/)")
        f_page(browser, base, errors, tool, a.shots)
        print("the demo's share link")
        demo_share(browser, base, errors)
        print("the gallery (/formulas/)")
        gallery(browser, base, errors, a.shots)
        print("the HTTP API (dev/api.html, dev/api/)")
        api_pages(browser, a.site, a.dist, a.art, errors)
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
