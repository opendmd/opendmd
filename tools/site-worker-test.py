# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright>=1.45"]
# ///
"""The opendmd-site Worker (site/worker/) over HTTP, against `cf dev`
serving a built site (make site):

    uv run tools/site-worker-test.py target/site [--base URL] [--smoke]

Without --base it starts `cf dev` itself (site/worker's pinned cf and
wrangler: make site-worker-deps) on a free port, and stops it after, with
the D1 migrations applied to cf dev's local database first (offline:
nothing in the account is touched). With --smoke, tools/site-smoke.py then
runs against the same server (--base).

- /api/health: 200 JSON {ok, site, version}, CORS *, not cached; HEAD,
  OPTIONS 204, POST 405 with Allow; another /api/ path 404 JSON;
- /f/ (the static Formula page) served through the Worker with its CORS and
  its CSP; an unknown short id /f/<id> answers the 404 page, never cached;
- short links: POST /api/f stores a Formula (201, its content address as
  the id, computed here too), the same source again (CRLF, trailing lines)
  200 with the same id; /api/f/<id> JSON with CORS *; /f/<id> the page with
  the Formula in #f-data, its name as the title and og:title, markup in the
  source only ever as text, the /f/ CSP; /f/<id>.psi the source; over
  4,096 bytes 413, not compiling 422 with the line, an invisible character
  422, not UTF-8 400, GET 405; another site's page 403 and no CORS on
  POST; in Chromium, /f/'s Copy link makes and copies the short link (and
  copies the long one when the site can't be reached), the short page
  plays it; past the rate limit 429 with Retry-After (last). Against a
  live site the Formulas posted are fixed ones (a rerun finds them: 200),
  locally each run's are new;
- every HTML page the site holds answers 200 (x.html after its one 307 to
  /x); an address with nothing there gets the 404 page with a 404;
- _redirects: every line a 301 to where it says, which answers 200;
- _headers: one CSP on every response, the right one per path; nosniff;
  no COOP/COEP; CORS * on install/, art-index/, f/; release.json for 60 s,
  the version-named release files immutable, the rest 600 s;
- www.opendmd.dev on a Worker path: 301 to https://opendmd.dev/… (against
  opendmd.dev only; skipped under cf dev and against a workers.dev URL);
- every request names itself in User-Agent: Cloudflare's Browser Integrity
  Check refuses urllib's default one (error 1010);
- _headers, _redirects and .assetsignore not served;
- in Chromium, no CSP violation on the docs, the install page (ESP Web
  Tools from jsDelivr), the gallery, the licences, the 404 page, the demo
  and /f/ with a Formula.
"""

import argparse
import hashlib
import json
import os
import re
import signal
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORKER = ROOT / "site/worker"
failures: list[str] = []


def check(ok: bool, what: str) -> None:
    print(f"  {'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        failures.append(what)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *a, **k):
        return None


OPENER = urllib.request.build_opener(NoRedirect)
# Cloudflare's Browser Integrity Check (error 1010) refuses urllib's default
# "Python-urllib/3.x" user agent with a 403; name the test instead.
UA = "opendmd-site-test/1 (+https://opendmd.dev)"


def get(base: str, path: str, method: str = "GET", headers: dict | None = None, data: bytes | None = None):
    """(status, headers, body) without following redirects."""
    req = urllib.request.Request(base + path, method=method, headers={"User-Agent": UA, **(headers or {})}, data=data)
    try:
        with OPENER.open(req, timeout=30) as r:
            return r.status, r.headers, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.headers, e.read()


def post(base: str, path: str, body: bytes, headers: dict | None = None):
    """POST body as text/plain: (status, headers, parsed JSON or {})."""
    st, h, raw = get(base, path, "POST", {"Content-Type": "text/plain; charset=utf-8", **(headers or {})}, body)
    try:
        d = json.loads(raw)
        return st, h, d if isinstance(d, dict) else {}
    except ValueError:
        return st, h, {}


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def start_dev(site: Path, port: int) -> subprocess.Popen:
    if not (WORKER / "node_modules/.bin/cf").exists():
        raise SystemExit("site/worker has no node_modules: make site-worker-deps")
    env = {**os.environ, "SITE_DIR": str(site.resolve()), "SITE_DEV_PORT": str(port),
           "SITE_VERSION": subprocess.run(["git", "describe", "--always", "--dirty", "--tags", "--match", "v*"], cwd=ROOT,
                                          capture_output=True, text=True).stdout.strip() or "dev"}
    log = open(ROOT / "target/site-dev.log", "w")
    # cf dev's local D1 (site/worker/.wrangler/state, wrangler's own place):
    # the migrations first, as make site-dev does.
    r = subprocess.run(["npx", "--no-install", "cf", "d1", "migrations", "apply", d1_id(), "--local", "--persist-to", ".wrangler/state"],
                       cwd=WORKER, env=env, stdout=log, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL)
    if r.returncode:
        raise SystemExit(f"cf d1 migrations apply --local failed ({r.returncode}); see target/site-dev.log")
    proc = subprocess.Popen(["npx", "--no-install", "cf", "dev"], cwd=WORKER, env=env, stdout=log, stderr=subprocess.STDOUT,
                            start_new_session=True)
    base = f"http://127.0.0.1:{port}"
    for _ in range(120):
        if proc.poll() is not None:
            raise SystemExit(f"cf dev exited ({proc.returncode}); see target/site-dev.log")
        try:
            if get(base, "/api/health")[0] == 200:
                print(f"cf dev: {base} (log: target/site-dev.log)")
                return proc
        except OSError:
            pass
        time.sleep(0.5)
    stop(proc)
    raise SystemExit("cf dev did not answer /api/health within 60 s; see target/site-dev.log")


def d1_id() -> str:
    """The DB binding's database id, from cloudflare.config.ts."""
    m = re.search(r'DB: \{[^}]*\bid: "([0-9a-f-]{36})"', (WORKER / "cloudflare.config.ts").read_text())
    if not m:
        raise SystemExit("cloudflare.config.ts: no DB binding with an id")
    return m.group(1)


def stop(proc: subprocess.Popen) -> None:
    try:
        os.killpg(proc.pid, signal.SIGTERM)
        proc.wait(timeout=15)
    except (ProcessLookupError, subprocess.TimeoutExpired):
        os.killpg(proc.pid, signal.SIGKILL)


def csp_of(h) -> list[str]:
    return h.get_all("Content-Security-Policy") or []


def api(base: str) -> None:
    st, h, body = get(base, "/api/health")
    d = json.loads(body) if st == 200 else {}
    check(st == 200 and d.get("ok") is True and d.get("site") == "opendmd-site" and isinstance(d.get("version"), str)
          and d["version"], f"/api/health: 200 {d}")
    check(h.get("Access-Control-Allow-Origin") == "*" and h.get("Cache-Control") == "no-store"
          and "json" in h.get("Content-Type", ""), "/api/health: JSON, CORS *, no-store")
    st, h, body = get(base, "/api/health", "HEAD")
    check(st == 200 and body == b"", f"/api/health: HEAD {st}")
    st, h, _ = get(base, "/api/health", "OPTIONS")
    check(st == 204 and "GET" in h.get("Access-Control-Allow-Methods", ""), f"/api/health: OPTIONS {st}")
    st, h, _ = get(base, "/api/health", "POST")
    check(st == 405 and "GET" in h.get("Allow", ""), f"/api/health: POST {st}, Allow: {h.get('Allow')}")
    st, h, body = get(base, "/api/nothing")
    check(st == 404 and json.loads(body) == {"error": "not found"}, f"another /api/ path: 404 JSON ({st})")


def formula(base: str) -> None:
    st, h, body = get(base, "/f/")
    check(st == 200 and b'src="f.js"' in body, f"/f/: the static Formula page through the Worker ({st})")
    csp = csp_of(h)
    check(h.get("Access-Control-Allow-Origin") == "*" and len(csp) == 1 and "'wasm-unsafe-eval'" in csp[0]
          and "connect-src 'self' http:" in csp[0], f"/f/: CORS * and its CSP {csp}")
    st, h, _ = get(base, "/f/f.js")
    check(st == 200 and h.get("Access-Control-Allow-Origin") == "*", f"/f/f.js: {st}, CORS {h.get('Access-Control-Allow-Origin')}")
    st, h, body = get(base, "/f/8m4kq2v9tz")
    check(st == 404 and b"Not found" in body and h.get("Cache-Control") == "no-store" and h.get("X-Robots-Tag") == "noindex",
          f"/f/<id> with nothing stored: the 404 page, never cached ({st}, {h.get('Cache-Control')})")
    st, h, body = get(base, "/f/no-such-file.js")
    check(st == 404 and h.get("Cache-Control") != "no-store", f"/f/<not an id>: a plain 404 ({st}, {h.get('Cache-Control')})")
    www(base)


ID_RE = r"[0-9a-hjkmnp-tv-z]{10,}"
CROCKFORD = "0123456789abcdefghjkmnpqrstvwxyz"


def content_id(src: str) -> str:
    """The id POST /api/f gives a source: 10 Crockford base32 characters of
    the SHA-256 of the source as stored (CRLF -> LF, trailing blank space cut)."""
    norm = re.sub(r"\s+$", "", src.replace("\r\n", "\n").replace("\r", "\n"))
    bits = "".join(f"{b:08b}" for b in hashlib.sha256(norm.encode()).digest())
    return "".join(CROCKFORD[int(bits[i:i + 5], 2)] for i in range(0, 50, 5))


class Short:
    """The Formulas this run posts: fixed ones against a live site (a rerun
    finds them stored: 200), new ones for each local run (201)."""

    def __init__(self, live: bool):
        self.live = live
        tag = "" if live else f"\n-- run {time.time_ns()}"
        self.a = 'name "Site test rings"\nloop 4\npal(rainbow, frac(len(x - 0.5, (y - 0.5)/4)*3 - t/4))' + tag
        self.markup = 'name "Site test markup"\nloop 4\ngray(x) -- </script><img src=x onerror="window.pwned=1"> & done' + tag
        self.b = 'control size = 0.3 "Size"\nloop 4\npal(fire, frac(len(x - 0.5, y - 0.5)*(1 + size) - t/4))' + tag
        self.b_name = "Site test slider"
        self.id_a = ""


def shortlinks(base: str, sh: Short) -> None:
    st, h, d = post(base, "/api/f", sh.a.encode())
    if st == 429 and sh.live:  # a run a moment ago used this minute's posts up
        print("  (rate limited by an earlier run: waiting a minute)")
        time.sleep(61)
        st, h, d = post(base, "/api/f", sh.a.encode())
    want = content_id(sh.a)
    new_ok = st == 201 and d.get("new") is True if not sh.live else (st, d and d.get("new")) in ((201, True), (200, False))
    check(new_ok and d["id"] == want and d.get("name") in ("Site test rings", None) and d["url"].endswith("/f/" + want)
          and h.get("Access-Control-Allow-Origin") is None, f"POST /api/f: {st} {d}, the content address {want}")
    sh.id_a = i = (d or {}).get("id", want)
    st, h, d = post(base, "/api/f", (sh.a.replace("\n", "\r\n") + "\r\n\n\n").encode())
    check(st == 200 and d == {"id": i, "url": d.get("url"), "new": False}, f"POST /api/f again (CRLF, blank lines after): 200, the same id ({st} {d})")
    st, h, body = get(base, f"/api/f/{i}")
    j = json.loads(body) if st == 200 else {}
    check(st == 200 and j.get("src") == sh.a and j.get("name") == "Site test rings" and j.get("id") == i and isinstance(j.get("esp32_us"), int)
          and h.get("Access-Control-Allow-Origin") == "*" and "json" in h.get("Content-Type", ""),
          f"/api/f/<id>: 200 JSON, CORS * ({st}, {sorted(j)})")
    st, h, _ = get(base, f"/api/f/{i}", "OPTIONS")
    check(st == 204 and h.get("Access-Control-Allow-Origin") == "*", f"/api/f/<id>: OPTIONS {st}, CORS *")
    st, h, body = get(base, f"/f/{i}.psi")
    check(st == 200 and body.decode() == sh.a + "\n" and h.get("Content-Type", "").startswith("text/plain")
          and h.get("Access-Control-Allow-Origin") == "*", f"/f/<id>.psi: the source as text ({st})")
    # The page: the Formula in it, its name the title, markup only as text.
    st, h, d = post(base, "/api/f", sh.markup.encode())
    m = (d or {}).get("id", "")
    st, h, body = get(base, f"/f/{m}")
    html = body.decode(errors="replace")
    csp = csp_of(h)
    check(st == 200 and "<title>Site test markup · OpenDMD</title>" in html and 'property="og:title" content="Site test markup"' in html
          and 'id="f-data"' in html and 'src="f.js"' in html, f"/f/<id>: the page with the Formula, its name the title ({st})")
    check("</script><img" not in html and "\\u003c/script>" in html and "&lt;/script&gt;&lt;img" in html,
          "/f/<id>: markup in the source only ever as text")
    check(len(csp) == 1 and "'wasm-unsafe-eval'" in csp[0] and h.get("Cache-Control") == "public, max-age=600"
          and h.get("X-Robots-Tag") == "noindex" and not h.get("ETag"), f"/f/<id>: the /f/ CSP, cached 600 s, noindex ({h.get('Cache-Control')})")
    # Refusals.
    big = sh.a + "\n-- " + "x" * 4096
    st, h, d = post(base, "/api/f", big.encode())
    check(st == 413 and "4096" in (d or {}).get("error", ""), f"POST /api/f over 4,096 bytes: 413 ({st} {d})")
    st, h, d = post(base, "/api/f", b"loop 4\ngray(x +)\n")
    check(st == 422 and (d or {}).get("line") == 2 and "line 2" in d.get("error", ""), f"POST /api/f that doesn't compile: 422, line 2 ({st} {d})")
    st, h, d = post(base, "/api/f", "loop 4\ngray(x) -- \u202eevil\n".encode())
    check(st == 422 and (d or {}).get("line") == 2, f"POST /api/f with a direction-changing character: 422 ({st} {d})")
    st, h, d = post(base, "/api/f", b"gray(x) -- \xff\xfe")
    check(st == 400, f"POST /api/f not UTF-8: 400 ({st} {d})")
    st, h, _ = get(base, "/api/f")
    check(st == 405 and h.get("Allow") == "POST", f"GET /api/f: 405, Allow: {h.get('Allow')}")
    # Another site's page: refused before the rate limit, and no CORS.
    st, h, d = post(base, "/api/f", sh.a.encode(), {"Origin": "http://opendmd.local", "Sec-Fetch-Site": "cross-site"})
    check(st == 403 and h.get("Access-Control-Allow-Origin") is None, f"POST /api/f from another site's page: 403, no CORS ({st})")
    st, h, _ = get(base, "/api/f", "OPTIONS", {"Origin": "http://opendmd.local", "Access-Control-Request-Method": "POST"})
    check(st == 204 and h.get("Access-Control-Allow-Origin") is None, f"OPTIONS /api/f: {st}, no CORS")
    st, h, body = get(base, "/f/zzzzzzzzzz")
    check(st == 404 and b"Not found" in body and h.get("Cache-Control") == "no-store", f"/f/<unknown id>: the 404 page, no-store ({st})")
    st, h, body = get(base, "/api/f/zzzzzzzzzz")
    check(st == 404 and json.loads(body).get("error"), f"/api/f/<unknown id>: 404 JSON ({st})")


def short_in_browser(base: str, sh: Short) -> None:
    """/f/'s Copy link makes the short link; the short page plays it."""
    from playwright.sync_api import sync_playwright
    long = f"{base}/f/#" + urllib.parse.urlencode({"formula": sh.b, "name": sh.b_name})
    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True)
        ctx = browser.new_context()
        ctx.grant_permissions(["clipboard-read", "clipboard-write"], origin=base)
        page = ctx.new_page()
        page.add_init_script("""document.addEventListener('securitypolicyviolation', e =>
            (window.__csp = window.__csp || []).push(e.violatedDirective + ' ' + e.blockedURI))""")
        errors: list[str] = []
        page.on("pageerror", lambda e: errors.append(str(e)))
        page.goto(long)
        page.wait_for_selector("#f:not([hidden]):not([aria-busy])", timeout=20000)
        page.click("#f-copy-link")
        clip = ""
        for _ in range(100):
            clip = page.evaluate("navigator.clipboard.readText()")
            if clip != "" and "#formula" not in clip:
                break
            page.wait_for_timeout(100)
        sid = clip.rsplit("/", 1)[-1]
        check(re.fullmatch(re.escape(base) + "/f/" + ID_RE, clip) is not None, f"/f/ in Chromium: Copy link copies a short link ({clip[:70]})")
        st, h, body = get(base, f"/api/f/{sid}")
        stored = json.loads(body).get("src", "") if st == 200 else ""
        check(stored == f'name "{sh.b_name}"\n' + sh.b, f"the short link's Formula is the page's, named by a name line ({stored[:40]!r}…)")
        page.goto(clip + "?a=0.7")
        page.wait_for_selector("#f:not([hidden]):not([aria-busy])", timeout=20000)
        for _ in range(100):
            if page.evaluate("+(document.querySelector('#f-panel').dataset.frames || 0) > 5"):
                break
            page.wait_for_timeout(100)
        check(page.inner_text("#f-name") == sh.b_name and page.locator("#f-code").inner_text().rstrip("\n") == stored
              and page.input_value("#f-sl-a") == "0.7" and page.evaluate("+(document.querySelector('#f-panel').dataset.frames || 0)") > 5,
              "the short page: the name, the code, the slider from ?a=, the preview playing")
        page.eval_on_selector("#f-sl-a", "r => { r.value = 0.5; r.dispatchEvent(new Event('input')); }")
        page.click("#f-copy-link")
        page.wait_for_timeout(300)
        clip2 = page.evaluate("navigator.clipboard.readText()")
        check(clip2 == clip + "?a=0.5", f"the short page's Copy link: the short link with the slider ({clip2[len(base):]})")
        page.wait_for_timeout(300)
        seen = page.evaluate("window.__csp || []")
        check(not seen and not errors and page.locator("#f-qr svg").count() == 1, f"the short page: no CSP violation or error {seen[:2]} {errors[:2]}")
        # Add to my panel: a top-level navigation to the display (remembered
        # here; never a real one) with the text, the slider and the mark.
        page.evaluate("localStorage.setItem('odmd.display', '192.168.1.50')")
        ctx.route("http://192.168.1.50/**", lambda r: r.fulfill(body="<p>display</p>", content_type="text/html"))
        with page.expect_navigation(url=re.compile(r"^http://192\.168\.1\.50/"), timeout=20000):
            page.click("#f-add")
        q = urllib.parse.parse_qs(urllib.parse.urlsplit(page.url).fragment)
        check(q == {"formula": [stored], "name": [sh.b_name], "a": ["0.5"], "add": ["1"]},
              f"the short page: Add to my panel hands its Formula to the display ({page.url[:60]}…)")
        ctx.close()
        # The site out of reach: the long link, which needs nothing from it.
        ctx = browser.new_context()
        ctx.grant_permissions(["clipboard-read", "clipboard-write"], origin=base)
        ctx.route("**/api/f", lambda r: r.abort())
        page = ctx.new_page()
        page.goto(long)
        page.wait_for_selector("#f:not([hidden]):not([aria-busy])", timeout=20000)
        page.click("#f-copy-link")
        page.wait_for_timeout(500)
        clip = page.evaluate("navigator.clipboard.readText()")
        check(clip.startswith(f"{base}/f/#formula=") and page.inner_text("#f-copy-link") == "Copied a long link",
              f"/f/ offline: Copy link copies the long link ({clip[:50]}…, {page.inner_text('#f-copy-link')!r})")
        ctx.close()
        browser.close()


def rate_limit(base: str, sh: Short) -> None:
    """Last: POST until the limiter says no (10 a minute per address; on
    Cloudflare per location and eventually consistent, so up to 60 tries)."""
    for n in range(1, 61):
        st, h, d = post(base, "/api/f", sh.a.encode())
        if st != 200:
            break
    check(st == 429 and h.get("Retry-After") == "60" and "minute" in (d or {}).get("error", ""),
          f"POST /api/f past the rate limit: 429 with Retry-After after {n} more posts ({st} {d})")


def www(base: str) -> None:
    """www.opendmd.dev -> the apex, on Cloudflare only: the zone's redirect
    rule answers every www path, and the Worker the /f/* and /api/* ones.
    cf dev cannot be asked as www (with `domains` set it serves every
    request as the first domain, whatever the Host), nor can a workers.dev
    preview: skipped there."""
    host = urllib.parse.urlsplit(base).hostname or ""
    if host in ("opendmd.dev", "www.opendmd.dev"):
        cases = [("https://www.opendmd.dev", {})]
    else:
        print(f"  skip www.opendmd.dev -> opendmd.dev: {host} is not opendmd.dev")
        return
    for b, hdr in cases:
        st, h, _ = get(b, "/f/8m4kq2v9tz", headers=hdr)
        check(st == 301 and h.get("Location") == "https://opendmd.dev/f/8m4kq2v9tz", f"www.opendmd.dev/f/…: 301 -> {h.get('Location')}")
        st, h, _ = get(b, "/api/health?x=1", headers=hdr)
        check(st == 301 and h.get("Location") == "https://opendmd.dev/api/health?x=1", f"www.opendmd.dev/api/…: 301 -> {h.get('Location')}")


def follow(base: str, path: str, limit: int = 5):
    """GET, following same-site redirects: (status, headers, body, hops)."""
    hops = 0
    st, h, body = get(base, path)
    while st in (301, 302, 307, 308) and hops < limit:
        loc = urllib.parse.urljoin(base + path, h["Location"])
        if not loc.startswith(base):
            break
        path = loc[len(base):] or "/"
        st, h, body = get(base, path)
        hops += 1
    return st, h, body, hops


def pages(base: str, site: Path) -> None:
    bad = []
    for p in sorted(site.rglob("*.html")):
        rel = "/" + p.relative_to(site).as_posix()
        if rel == "/404.html":
            continue
        st, h, body, hops = follow(base, rel)
        # x.html 307s to /x and x/index.html to x/ (html_handling
        # auto-trailing-slash), a MOVED page 301s: one hop at most.
        if st != 200 or not body or hops > 1:
            bad.append(f"{rel} {st} after {hops} redirects")
    n = sum(1 for _ in site.rglob("*.html")) - 1
    check(not bad, f"every page answers 200 ({n} pages){': ' + ', '.join(bad[:8]) if bad else ''}")
    st, h, body = get(base, "/no/such/page")
    check(st == 404 and b"<title>Not found" in body and b'href="/site.css"' in body,
          f"an address with nothing there: the 404 page with absolute links ({st})")
    hidden = [f for f in ("/_headers", "/_redirects", "/.assetsignore") if get(base, f)[0] != 404]
    check(not hidden, f"the Cloudflare files are not served {hidden}")


def redirects(base: str, site: Path) -> None:
    bad = []
    lines = [ln.split() for ln in (site / "_redirects").read_text().splitlines() if ln.strip() and not ln.startswith("#")]
    for old, new, code in lines:
        st, h, _ = get(base, old)
        if st != int(code) or h.get("Location") != new and urllib.parse.urlsplit(h.get("Location", "")).path != new:
            bad.append(f"{old}: {st} {h.get('Location')}")
            continue
        if get(base, new)[0] != 200:  # straight to the served address: no second hop
            bad.append(f"{old} -> {new}: not 200")
    check(lines and not bad, f"_redirects: {len(lines)} old addresses, each a 301 to a page that answers 200"
                             f"{': ' + '; '.join(bad[:6]) if bad else ''}")


def headers(base: str, site: Path) -> None:
    ver = json.loads((site / "install/release.json").read_text())["version"]
    want = {  # path: (CSP must contain, must not contain, CORS, Cache-Control)
        "/": ("script-src 'self';", "wasm", None, "public, max-age=600"),
        "/guide/web-app": ("script-src 'self';", "wasm", None, "public, max-age=600"),
        "/formulas/": ("script-src 'self';", "jsdelivr", None, "public, max-age=600"),
        "/install/": ("script-src 'self' https://cdn.jsdelivr.net", "wasm", "*", "public, max-age=600"),
        "/install/release.json": (None, None, "*", "public, max-age=60"),
        f"/install/firmware/opendmd-{ver}-4mb.odfw": (None, None, "*", "public, max-age=31536000, immutable"),
        f"/install/firmware/opendmd-{ver}-www.pack": (None, None, "*", "public, max-age=31536000, immutable"),
        "/install/firmware/manifest-4mb.json": (None, None, "*", "public, max-age=600"),
        "/install/firmware/updater.bin": (None, None, "*", "public, max-age=600"),
        "/art-index/v1/GB.json": (None, None, "*", "public, max-age=600"),
        "/demo/": ("connect-src 'self' https://gifcities.archive.org", None, None, "public, max-age=600"),
        "/demo/app.js": (None, None, None, "public, max-age=600"),
        "/f/": ("'wasm-unsafe-eval'", "jsdelivr", "*", "public, max-age=600"),
    }
    bad = []
    for path, (has, hasnt, cors, cache) in want.items():
        st, h, _ = get(base, path)
        csp = csp_of(h)
        why = []
        if st != 200:
            why.append(f"status {st}")
        if len(csp) != 1:
            why.append(f"{len(csp)} CSPs")
        elif (has and has not in csp[0]) or (hasnt and hasnt in csp[0]) or "frame-ancestors 'none'" not in csp[0]:
            why.append(f"CSP {csp[0][:80]}…")
        if h.get("Access-Control-Allow-Origin") != cors:
            why.append(f"CORS {h.get('Access-Control-Allow-Origin')}")
        if h.get("Cache-Control") != cache:
            why.append(f"Cache-Control {h.get('Cache-Control')}")
        if h.get("X-Content-Type-Options") != "nosniff" or h.get("Referrer-Policy") != "strict-origin-when-cross-origin":
            why.append("nosniff/Referrer-Policy")
        if "serial=(self)" not in (h.get("Permissions-Policy") or ""):
            why.append("Permissions-Policy")
        if h.get("Cross-Origin-Opener-Policy") or h.get("Cross-Origin-Embedder-Policy"):
            why.append("COOP/COEP")
        if why:
            bad.append(f"{path}: {', '.join(why)}")
    check(not bad, f"_headers: CSP, CORS, caching and the rest on {len(want)} paths{': ' + '; '.join(bad) if bad else ''}")


def csp_in_browser(base: str, short_id: str) -> None:
    from playwright.sync_api import sync_playwright
    paths = ["/", "/guide/web-app", "/install/", "/formulas/", "/licenses/", "/dev/api", "/no/such/page", "/demo/",
             "/f/#" + urllib.parse.urlencode({"formula": "loop 4\npal(rainbow, frac(x + t/4))", "name": "Rainbow"}),
             f"/f/{short_id}"]
    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True)
        for path in paths:
            ctx = browser.new_context()
            page = ctx.new_page()
            seen: list[str] = []
            page.add_init_script("""document.addEventListener('securitypolicyviolation', e =>
                (window.__csp = window.__csp || []).push(e.violatedDirective + ' ' + e.blockedURI))""")
            page.on("console", lambda m: m.type == "error" and "Content Security Policy" in m.text and seen.append(m.text[:160]))
            page.goto(base + path, wait_until="load")
            if path.startswith("/f/"):
                page.wait_for_selector("#f:not([hidden]):not([aria-busy])", timeout=20000)
            elif path == "/demo/":
                page.wait_for_timeout(1500)
            elif path == "/install/":
                # (polled with evaluate: wait_for_function evals, which the CSP refuses)
                for _ in range(200):
                    if page.evaluate("!!customElements.get('esp-web-install-button')"):
                        break
                    page.wait_for_timeout(100)
                else:
                    seen.append("ESP Web Tools' button never defined")
            page.wait_for_timeout(300)
            seen += page.evaluate("window.__csp || []")
            check(not seen, f"CSP in Chromium: {path.split('#')[0]} {'clean' if not seen else seen[:3]}")
            ctx.close()
        browser.close()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("site", type=Path, help="make site's output, the folder cf dev serves")
    ap.add_argument("--base", help="test this server instead of starting cf dev")
    ap.add_argument("--port", type=int, default=0, help="cf dev's port (default: a free one)")
    ap.add_argument("--smoke", action="store_true", help="then run tools/site-smoke.py --base against it")
    ap.add_argument("--dist", type=Path, default=ROOT / "dist")
    ap.add_argument("--art", type=Path, default=ROOT / "target/site-art")
    a = ap.parse_args()
    if not (a.site / "_headers").is_file():
        raise SystemExit(f"{a.site} has no _headers: make site first")
    proc = None
    if a.base:
        base = a.base.rstrip("/")
    else:
        port = a.port or free_port()
        proc = start_dev(a.site, port)
        base = f"http://127.0.0.1:{port}"
    smoke = 0
    try:
        print("the Worker's routes")
        api(base)
        formula(base)
        live = urllib.parse.urlsplit(base).hostname not in ("127.0.0.1", "localhost")
        sh = Short(live)
        print(f"short links ({'live: fixed Formulas' if live else 'local: new Formulas'})")
        shortlinks(base, sh)
        short_in_browser(base, sh)
        print("pages, redirects, headers")
        pages(base, a.site)
        redirects(base, a.site)
        headers(base, a.site)
        print("CSP in the browser")
        csp_in_browser(base, sh.id_a)
        print("the rate limit")
        rate_limit(base, sh)
        print(f"site-worker-test: {'ok' if not failures else f'{len(failures)} failures'}")
        if a.smoke:
            print(f"site-smoke --base {base}")
            smoke = subprocess.run(["uv", "run", str(ROOT / "tools/site-smoke.py"), str(a.site), "--base", base,
                                    "--dist", str(a.dist), "--art", str(a.art)], cwd=ROOT).returncode
    finally:
        if proc:
            stop(proc)
    return 1 if failures or smoke else 0


if __name__ == "__main__":
    sys.exit(main())
