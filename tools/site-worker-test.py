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
- /f/ (the Formula page) a static asset with its CSP, the Worker not in
  the way; no anonymous short links: POST /api/f and /api/f/<id> 404 JSON,
  /f/<id> and /f/<id>.psi the 404 page; the gallery's files,
  /formulas/src/<slug>.psi, CORS * (a panel's web app fetches them: Open on
  my panel);
- every HTML page the site holds answers 200 (x.html after its one 307 to
  /x); an address with nothing there gets the 404 page with a 404;
- _redirects: every line a 301 to where it says, which answers 200;
- _headers: one CSP on every response, the right one per path; nosniff;
  no COOP/COEP; CORS * on install/, art-index/, formulas/src/; release.json for 60 s,
  the version-named release files immutable, the rest 600 s;
- www.opendmd.dev on a Worker path: 301 to https://opendmd.dev/… (against
  opendmd.dev only; skipped under cf dev and against a workers.dev URL);
- every request names itself in User-Agent: Cloudflare's Browser Integrity
  Check refuses urllib's default one (error 1010);
- _headers, _redirects and .assetsignore not served;
- in Chromium, the home page's live panels (the hero's Endless road and
  the Formulas door's Matrix): each poster first, then a canvas drawing lit,
  changing pixels with no CSP violation or console error; with reduced
  motion the stills stay and no wasm loads; with the wasm blocked the
  stills stay; the gallery's tiles: hovering one plays it (changing
  pixels), leaving brings its thumbnail back, keyboard focus plays one
  too, one at a time; screenshots in target/site-worker-test/ (--shots);
- in Chromium, no CSP violation on the docs, the install page (ESP Web
  Tools from jsDelivr), the gallery, the licences, the 404 page, the demo
  and /f/ with a gallery Formula;
- the gallery store (S6): locally, against a fixture of the public repo
  served here (git's info/refs and a tarball, as GitHub serves them; cf dev
  pointed at it with SITE_TEST_GALLERY_SOURCE) and a wiped local store:
  the built page until the store is ready; a simulated cron run syncs the
  fixture (good contributions, one identical to an example, and files that
  must be refused: not compiling, a reserved author, CRLF with a bidi
  character, a bad name, a link, a folder, an executable); every verdict,
  sentence and field the same as opendmd-gallery's on the same files,
  every thumbnail the same pixels; /formulas/ the page gallery_page.py
  builds from the same files (X-Gallery: store), its src/ and thumbs/ from
  the store; then POST /api/gallery/sync with a contribution broken in
  place (the last good version kept, the refusal listed), one removed (gone
  from the page, src/ and thumbs/) and one added, a second POST at once
  429; last the fixture emptied (every contribution gone: the page as built
  without them). Live: POST /api/gallery/sync, the commit GitHub says main
  is, this build's examples, the contributions opendmd-gallery passes in
  that commit's tarball, and the page as gallery_page.py builds it.
"""

import argparse
import hashlib
import http.server
import io
import json
import os
import re
import shutil
import signal
import socket
import sqlite3
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
import zlib
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORKER = ROOT / "site/worker"
sys.path.insert(0, str(ROOT / "tools"))
import gallery_page  # noqa: E402  (tools/gallery_page.py: the built gallery page)
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


def start_dev(site: Path, port: int, gallery_source: str) -> subprocess.Popen:
    if not (WORKER / "node_modules/.bin/cf").exists():
        raise SystemExit("site/worker has no node_modules: make site-worker-deps")
    env = {**os.environ, "SITE_DIR": str(site.resolve()), "SITE_DEV_PORT": str(port), "SITE_TEST_GALLERY_SOURCE": gallery_source,
           "SITE_VERSION": subprocess.run(["git", "describe", "--always", "--dirty", "--tags", "--match", "v*"], cwd=ROOT,
                                          capture_output=True, text=True).stdout.strip() or "dev"}
    log = open(ROOT / "target/site-dev.log", "w")
    # cf dev's local D1 (site/worker/.wrangler/state, wrangler's own place):
    # the migrations first, as make site-dev does.
    r = subprocess.run(["npx", "--no-install", "cf", "d1", "migrations", "apply", d1_id(), "--local", "--persist-to", ".wrangler/state"],
                       cwd=WORKER, env=env, stdout=log, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL)
    if r.returncode:
        raise SystemExit(f"cf d1 migrations apply --local failed ({r.returncode}); see target/site-dev.log")
    # The gallery store from nothing: the local database's file (cf d1
    # query has no --local), before cf dev opens it.
    wiped = 0
    for db in (WORKER / ".wrangler/state/v3/d1").rglob("*.sqlite"):
        with sqlite3.connect(db) as c:
            if c.execute("select 1 from sqlite_master where name = 'gallery'").fetchone():
                c.executescript("delete from gallery; delete from gallery_refused; delete from meta;")
                wiped += 1
    if wiped != 1:
        raise SystemExit(f"{wiped} local D1 databases with a gallery table under site/worker/.wrangler/state: expected 1")
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
    check(st == 200 and b'src="f.js"' in body, f"/f/: the Formula page ({st})")
    csp = csp_of(h)
    check(h.get("Access-Control-Allow-Origin") is None and len(csp) == 1 and "'wasm-unsafe-eval'" in csp[0]
          and "connect-src" not in csp[0], f"/f/: its CSP, no CORS, nothing to connect to {csp}")
    # No anonymous short links (2026-10-30): nothing posts, nothing is stored.
    st, h, d = post(base, "/api/f", b"gray(x)\n")
    check(st == 404 and d == {"error": "not found"}, f"POST /api/f: 404, no short links ({st} {d})")
    st, h, body = get(base, "/api/f/8m4kq2v9tz")
    check(st == 404 and json.loads(body) == {"error": "not found"}, f"/api/f/<id>: 404 ({st})")
    for path in ("/f/8m4kq2v9tz", "/f/8m4kq2v9tz.psi"):
        st, h, body = get(base, path)
        check(st == 404 and b"Not found" in body, f"{path}: the 404 page ({st})")
    st, h, body = get(base, "/formulas/src/fire.psi")
    check(st == 200 and h.get("Access-Control-Allow-Origin") == "*" and body.startswith(b"-- author:"),
          f"/formulas/src/<slug>.psi: the file, CORS * ({st}, {h.get('Access-Control-Allow-Origin')})")
    www(base)


def www(base: str) -> None:
    """www.opendmd.dev -> the apex, on Cloudflare only: the zone's redirect
    rule answers every www path, and the Worker the /api/* and /formulas/* ones.
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
        st, h, _ = get(b, "/formulas/src/fire.psi", headers=hdr)
        check(st == 301 and h.get("Location") == "https://opendmd.dev/formulas/src/fire.psi", f"www.opendmd.dev/formulas/…: 301 -> {h.get('Location')}")
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
    # The menus and the footer as served: every link answers, straight away
    # (a folder's index is linked as the folder, so no redirect).
    st, h, body = get(base, "/guide/web-app")
    chrome = re.findall(r'<(?:header|nav|footer)\b.*?</(?:header|nav|footer)>', body.decode(), re.S)
    links = sorted({urllib.parse.urljoin("/guide/web-app", u) for c in chrome for u in re.findall(r'href="([^"#]+)"', c)
                    if not re.match(r"[a-z]+:", u)})
    slow = []
    for u in links:
        st_u, _, _, hops = follow(base, u)
        # (x.html 307s once to /x: the docs' own links, as a plain server needs them)
        if st_u != 200 or (hops and not u.endswith(".html")):
            slow.append(f"{u} {st_u} after {hops}")
    check(st == 200 and len(links) >= 15 and not slow, f"the menus' and the footer's {len(links)} links answer 200{': ' + ', '.join(slow[:6]) if slow else ''}")
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
    guessed = {"/gallery": "/formulas/", "/guide/install": "/install/", "/docs/index.html": "/guide/"}
    for old, new in guessed.items():
        if [old, new, "301"] not in lines:
            bad.append(f"{old}: not redirected to {new}")
    check(lines and not bad, f"_redirects: {len(lines)} old addresses, each a 301 to a page that answers 200"
                             f"{': ' + '; '.join(bad[:6]) if bad else ''}")


def headers(base: str, site: Path) -> None:
    ver = json.loads((site / "install/release.json").read_text())["version"]
    want = {  # path: (CSP must contain, must not contain, CORS, Cache-Control)
        # The home page's live panels and the gallery's tiles on hover: Psi's wasm, nothing more.
        "/": ("script-src 'self' 'wasm-unsafe-eval';", "connect-src", None, "public, max-age=600"),
        "/guide/web-app": ("script-src 'self' 'wasm-unsafe-eval';", "connect-src", None, "public, max-age=600"),
        "/guide/": ("script-src 'self' 'wasm-unsafe-eval';", "connect-src", None, "public, max-age=600"),
        # (60 s once the gallery store serves it; 600 s as built)
        "/formulas/": ("script-src 'self' 'wasm-unsafe-eval';", "connect-src", None, ("public, max-age=60", "public, max-age=600")),
        "/install/": ("script-src 'self' https://cdn.jsdelivr.net", "wasm", "*", "public, max-age=600"),
        "/install/release.json": (None, None, "*", "public, max-age=60"),
        f"/install/firmware/opendmd-{ver}-4mb.odfw": (None, None, "*", "public, max-age=31536000, immutable"),
        f"/install/firmware/opendmd-{ver}-www.pack": (None, None, "*", "public, max-age=31536000, immutable"),
        "/install/firmware/manifest-4mb.json": (None, None, "*", "public, max-age=600"),
        "/install/firmware/updater.bin": (None, None, "*", "public, max-age=600"),
        "/art-index/v1/GB.json": (None, None, "*", "public, max-age=600"),
        "/demo/": ("connect-src 'self' https://gifcities.archive.org", None, None, "public, max-age=600"),
        "/demo/app.js": (None, None, None, "public, max-age=600"),
        "/f/": ("'wasm-unsafe-eval'", "connect-src", None, "public, max-age=600"),
        # Create Art: the Studio's wasm, its sources' pictures, a picture from any https address.
        "/create/": ("connect-src 'self' https:", None, None, "public, max-age=600"),
        # The Psi reference's live examples: wasm, and no connect-src beyond the docs'.
        "/dev/psi/": ("script-src 'self' 'wasm-unsafe-eval';", "connect-src", None, "public, max-age=600"),
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
        if h.get("Cache-Control") not in ((cache,) if isinstance(cache, str) else cache):
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


def csp_in_browser(base: str) -> None:
    from playwright.sync_api import sync_playwright
    paths = ["/", "/guide/", "/guide/web-app", "/install/", "/formulas/", "/licenses/", "/dev/", "/dev/api", "/dev/psi/",
             "/no/such/page", "/demo/",
             "/f/#gallery=fire", "/create/"]
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
            elif path == "/create/":
                page.wait_for_selector("#create:not([aria-busy]) #studio", timeout=20000)
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


PIXELS = """sel => { const c = document.querySelector(sel); if (!c) return null;
  const d = c.getContext('2d').getImageData(0, 0, 128, 32).data; let lit = 0, h = 0;
  for (let i = 0; i < d.length; i += 4) { if (d[i] + d[i + 1] + d[i + 2] > 30) lit++; h = (h * 31 + d[i] + d[i + 1] * 7 + d[i + 2] * 13) | 0; }
  return { lit, h, frames: +(c.dataset.frames || 0) }; }"""


def live_panels(base: str, shots: Path) -> None:
    """The home page's live panels and the gallery's tiles on hover (psiled.js)."""
    from playwright.sync_api import sync_playwright
    shots.mkdir(parents=True, exist_ok=True)

    def watch(page) -> tuple[list[str], list[str]]:
        seen: list[str] = []
        psi: list[str] = []
        page.add_init_script("""document.addEventListener('securitypolicyviolation', e =>
            (window.__csp = window.__csp || []).push(e.violatedDirective + ' ' + e.blockedURI))""")
        # (A request the test blocks is logged by Chromium itself: "Failed to load resource";
        # the test's own reading of the canvas, PIXELS, draws Chromium's getImageData hint.)
        page.on("console", lambda m: m.type in ("error", "warning") and "Failed to load resource" not in m.text
                and not m.text.startswith("Canvas2D: Multiple readback") and seen.append(m.text[:160]))
        page.on("pageerror", lambda e: seen.append(str(e)[:160]))
        page.on("request", lambda r: "/pkg-psi/" in r.url and psi.append(r.url))
        return seen, psi

    def poll(page, expr: str, arg=None, timeout: float = 20000):
        for _ in range(int(timeout / 100)):
            v = page.evaluate(expr, arg)
            if v:
                return v
            page.wait_for_timeout(100)
        return None

    hero, door = ".hero-panel [data-live=on] canvas", ".door[data-live=on] canvas"
    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True)
        # The home page, live.
        ctx = browser.new_context(viewport={"width": 1280, "height": 1100})
        page = ctx.new_page()
        seen, psi = watch(page)
        page.goto(base + "/", wait_until="domcontentloaded")
        poster = page.evaluate("!!document.querySelector('.hero-panel .led img') && !document.querySelector('.hero-panel canvas')")
        a = poll(page, f"() => {{ const f = ({PIXELS}); const x = f('{hero}'); return x && x.frames >= 30 && x.lit > 200 && x; }}")
        page.wait_for_timeout(300)
        b = page.evaluate(PIXELS, hero)
        # Psi's files only once the page had loaded (their resource timings after its load event).
        early = page.evaluate("""() => { const load = performance.getEntriesByType('navigation')[0].loadEventStart;
            return performance.getEntriesByType('resource').filter(e => e.name.includes('/pkg-psi/') && e.startTime < load).length; }""")
        check(poster and psi and early == 0, f"home: the hero's still first, Psi loaded only after the page ({len(psi)} requests, {early} early)")
        check(bool(a and b and b["h"] != a["h"] and b["frames"] > a["frames"]),
              f"home: the hero plays Endless road live (lit {a and a['lit']} of 4096, frames {a and a['frames']} -> {b and b['frames']})")
        d1 = poll(page, f"() => {{ const f = ({PIXELS}); const x = f('{door}'); return x && x.frames >= 10 && x.lit > 20 && x; }}")
        page.wait_for_timeout(300)
        d2 = page.evaluate(PIXELS, door)
        check(bool(d1 and d2 and d1["h"] != d2["h"]), f"home: the Formulas door plays Matrix live ({d1 and d1['lit']} lit)")
        page.locator(".hero-panel").screenshot(path=str(shots / "home-hero-live.png"))
        page.locator(".doors").screenshot(path=str(shots / "home-get-started.png"))
        # Off-screen: the hero stops drawing.
        page.evaluate("scrollTo(0, document.body.scrollHeight)")
        page.wait_for_timeout(400)
        f1 = page.evaluate(PIXELS, hero)["frames"]
        page.wait_for_timeout(500)
        f2 = page.evaluate(PIXELS, hero)["frames"]
        check(f1 == f2, f"home: the hero pauses off-screen ({f1} -> {f2} frames)")
        seen += page.evaluate("window.__csp || []")
        check(not seen, f"home: no CSP violation or console message {seen[:3]}")
        ctx.close()
        # Reduced motion: the stills, no wasm.
        ctx = browser.new_context(reduced_motion="reduce")
        page = ctx.new_page()
        seen, psi = watch(page)
        page.goto(base + "/", wait_until="load")
        page.wait_for_timeout(2500)
        still = page.evaluate("[!!document.querySelector('.hero-panel .led img'), document.querySelectorAll('.hero-panel canvas, .door canvas').length]")
        check(still == [True, 0] and not psi and not seen, f"home, reduced motion: the stills stay, no wasm ({len(psi)} requests) {seen[:2]}")
        ctx.close()
        # The wasm out of reach: the stills stay, quietly.
        ctx = browser.new_context()
        page = ctx.new_page()
        seen, psi = watch(page)
        page.route("**/pkg-psi/**", lambda r: r.abort())
        page.goto(base + "/", wait_until="load")
        page.wait_for_timeout(3000)
        still = page.evaluate("[!!document.querySelector('.hero-panel .led img'), document.querySelectorAll('.hero-panel canvas').length]")
        seen += page.evaluate("window.__csp || []")
        check(still == [True, 0] and psi and not seen, f"home, no wasm: the still stays, nothing on the console {seen[:2]}")
        ctx.close()
        # The gallery: a tile plays while hovered or focused.
        ctx = browser.new_context(viewport={"width": 1280, "height": 900})
        page = ctx.new_page()
        seen, psi = watch(page)
        page.goto(base + "/formulas/", wait_until="load")
        page.wait_for_timeout(1500)
        check(not psi, f"gallery: no wasm before a hover ({len(psi)} requests)")
        tile = page.locator("li.ftile:has(a.fpic[href*='gallery=matrix'])")
        tile.scroll_into_view_if_needed()
        tile.hover()
        live = "li.ftile[data-live=on] canvas"
        a = poll(page, f"() => {{ const f = ({PIXELS}); const x = f('{live}'); return x && x.frames >= 10 && x.lit > 20 && x; }}")
        page.wait_for_timeout(300)
        b = page.evaluate(PIXELS, live)
        tile.screenshot(path=str(shots / "gallery-hover.png"))
        check(bool(a and b and a["h"] != b["h"]) and page.locator("li.ftile canvas").count() == 1,
              f"gallery: hovering Matrix's tile plays it ({a and a['frames']} -> {b and b['frames']} frames, one canvas)")
        page.mouse.move(2, 2)
        page.wait_for_timeout(200)
        back = page.evaluate("[document.querySelectorAll('li.ftile canvas').length, document.querySelector(\"li.ftile:has(a.fpic[href*='gallery=matrix']) .led img\").closest('.led').hidden]")
        check(back == [0, False], f"gallery: leaving it brings the thumbnail back {back}")
        other = page.locator("li.ftile:has(a.fpic[href*='gallery=starfield'])")
        other.locator("a.cta.alt").first.focus()
        f = poll(page, f"() => {{ const f = ({PIXELS}); const x = f('{live}'); return x && x.frames >= 5 && x; }}")
        check(bool(f) and page.locator("li.ftile canvas").count() == 1 and other.get_attribute("data-live") == "on",
              "gallery: keyboard focus in Starfield's tile plays it, one at a time")
        page.evaluate("document.activeElement.blur()")
        page.wait_for_timeout(200)
        check(page.locator("li.ftile canvas").count() == 0, "gallery: focus gone, the thumbnail back")
        seen += page.evaluate("window.__csp || []")
        check(not seen, f"gallery: no CSP violation or console message {seen[:3]}")
        ctx.close()
        browser.close()


# ------------------------------------------------------------------ the gallery store (S6)

REPO = "https://github.com/opendmd/opendmd.github.io"
HEADER = "-- author: Fixture Author\n-- licence: CC0-1.0\n"


class FixtureRepo:
    """The public gallery repo as the Worker reads it from GitHub: git's ref
    advertisement (`<repo>.git/info/refs?service=git-upload-pack`) naming
    main's commit, and that commit's tarball (`<repo>/archive/<sha>.tar.gz`,
    one top folder `<repo>-<sha>/`). Each state is a dict of gallery/ paths:
    text (a file), ("link", target), ("dir",), ("exec", text)."""

    def __init__(self):
        self.tars: dict[str, bytes] = {}
        self.head = ""
        repo = self

        class H(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                if self.path == "/repo.git/info/refs?service=git-upload-pack":
                    def pkt(s: bytes) -> bytes:
                        return f"{len(s) + 4:04x}".encode() + s
                    body = (pkt(b"# service=git-upload-pack\n") + b"0000" + pkt(f"{repo.head} HEAD\0multi_ack symref=HEAD:refs/heads/main\n".encode())
                            + pkt(f"{'1' * 40} refs/heads/other\n".encode()) + pkt(f"{repo.head} refs/heads/main\n".encode()) + b"0000")
                    ctype = "application/x-git-upload-pack-advertisement"
                else:
                    m = re.fullmatch(r"/repo/archive/([0-9a-f]{40})\.tar\.gz", self.path)
                    if not m or m.group(1) not in repo.tars:
                        self.send_error(404)
                        return
                    body, ctype = repo.tars[m.group(1)], "application/x-gzip"
                self.send_response(200)
                self.send_header("Content-Type", ctype)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *a):
                pass

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}/repo"

    def set(self, files: dict) -> str:
        """Make `files` main's commit: its sha."""
        buf = io.BytesIO()
        sha = hashlib.sha1(json.dumps(sorted((k, str(v)) for k, v in files.items())).encode()).hexdigest()
        top = f"opendmd.github.io-{sha}"
        with tarfile.open(fileobj=buf, mode="w:gz", format=tarfile.PAX_FORMAT) as t:
            def add(name: str, kind=tarfile.REGTYPE, data: bytes = b"", mode: int = 0o664, link: str = ""):
                i = tarfile.TarInfo(f"{top}/{name}")
                i.type, i.mode, i.size, i.linkname = kind, mode, len(data), link
                t.addfile(i, io.BytesIO(data) if kind == tarfile.REGTYPE else None)
            add("", tarfile.DIRTYPE, mode=0o775)
            add("README.md", data=b"# the gallery\n")
            add("gallery/", tarfile.DIRTYPE, mode=0o775)
            add("gallery/README.md", data=b"how to contribute\n")
            for name, v in sorted(files.items()):
                if isinstance(v, str):
                    add(f"gallery/{name}", data=v.encode())
                elif v[0] == "link":
                    add(f"gallery/{name}", tarfile.SYMTYPE, link=v[1], mode=0o777)
                elif v[0] == "dir":
                    add(f"gallery/{name}/", tarfile.DIRTYPE, mode=0o775)
                    add(f"gallery/{name}/inside.psi", data=b"-- author: x\n")
                elif v[0] == "exec":
                    add(f"gallery/{name}", data=v[1].encode(), mode=0o775)
        self.tars[sha] = buf.getvalue()
        self.head = sha
        return sha


def png_pixels(data: bytes) -> bytes:
    """A PNG's RGB888 pixels (8-bit RGB, any filter)."""
    assert data[:8] == b"\x89PNG\r\n\x1a\n", "not a PNG"
    i, idat, w, h = 8, b"", 0, 0
    while i < len(data):
        n = int.from_bytes(data[i:i + 4], "big")
        kind, body = data[i + 4:i + 8], data[i + 8:i + 8 + n]
        i += 12 + n
        if kind == b"IHDR":
            w, h = int.from_bytes(body[:4], "big"), int.from_bytes(body[4:8], "big")
            assert body[8:10] == b"\x08\x02", "not 8-bit RGB"
        elif kind == b"IDAT":
            idat += body
    raw, out, stride = zlib.decompress(idat), bytearray(), w * 3
    prev = bytearray(stride)
    for y in range(h):
        f, row = raw[y * (stride + 1)], bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for x in range(stride):
            a = row[x - 3] if x >= 3 else 0
            b, c = prev[x], prev[x - 3] if x >= 3 else 0
            if f == 1:
                row[x] = (row[x] + a) & 255
            elif f == 2:
                row[x] = (row[x] + b) & 255
            elif f == 3:
                row[x] = (row[x] + (a + b) // 2) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                row[x] = (row[x] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        out += row
        prev = row
    return bytes(out)


def native(tool: Path, folder: Path) -> tuple[dict, dict]:
    """opendmd-gallery check over a folder's .psi files (regular, not
    executable: the Worker refuses the rest before checking): ({slug:
    entry}, {slug: problems}), the thumbnails in folder/thumbs/."""
    files = sorted(p for p in folder.glob("*.psi") if p.is_file() and not p.is_symlink() and not os.access(p, os.X_OK))
    good, bad = {}, {}
    if files:
        r = subprocess.run([str(tool), "check", "--json", "--thumbs", str(folder / "thumbs"), *map(str, files)], capture_output=True, text=True)
        assert r.returncode in (0, 1), r.stderr
        for line in r.stdout.splitlines():
            e = json.loads(line)
            if "problems" in e:
                bad[e["file"][:-4]] = e["problems"]
            else:
                good[e["slug"]] = e
    return good, bad


def write_state(files: dict, folder: Path) -> None:
    """A fixture state as files on disk, as a checkout would have them."""
    shutil.rmtree(folder, ignore_errors=True)
    folder.mkdir(parents=True)
    for name, v in files.items():
        p = folder / name
        if isinstance(v, str):
            p.write_text(v)
        elif v[0] == "link":
            p.symlink_to(v[1])
        elif v[0] == "dir":
            p.mkdir()
        elif v[0] == "exec":
            p.write_text(v[1])
            p.chmod(0o755)


def lists_of(html: str) -> tuple[str, str]:
    """The page's two lists, the thumbnails' version queries cut."""
    def part(name):
        m = re.search(f"<!-- gallery:{name} -->(.*?)<!-- gallery:{name}-end -->", html, re.S)
        return re.sub(r'(thumbs/[a-z0-9-]+\.png)\?v=[0-9a-f]{12}', r"\1", m.group(1)) if m else None
    return part("community"), part("examples")


def gallery_api(base: str) -> dict:
    st, h, body = get(base, "/api/gallery")
    return json.loads(body) if st == 200 else {}


def compare_store(base: str, site: Path, tool: Path, shown: Path, refused_want: dict, what: str, tmp: Path) -> dict:
    """The store against opendmd-gallery and gallery_page.py over `shown`
    (the contributions the page should list) and `refused_want` (slug:
    problems): entries, refusals, the page's lists, thumbnails, src/."""
    d = gallery_api(base)
    good, bad = native(tool, shown)
    stored = {e["slug"]: e for e in d.get("entries", []) if e["source"] == "gallery"}
    fields = ("name", "about", "author", "licence", "look", "controls", "src", "loop_ms", "vm_us", "psb_len", "abi")
    diff = [f"{s}.{k}: {stored.get(s, {}).get(k)!r} != {e[k]!r}" for s, e in good.items() for k in fields if stored.get(s, {}).get(k) != e[k]]
    diff += [f"{s}.us: {stored[s]['esp32_us']} != {e['us']}" for s, e in good.items() if s in stored and stored[s]["esp32_us"] != e["us"]]
    check(set(stored) == set(good) and not diff and not bad,
          f"{what}: the store's contributions are opendmd-gallery's, field for field ({sorted(stored)}){': ' + '; '.join(diff[:4]) if diff else ''}")
    got_bad = {x["slug"]: x["problems"] for x in d.get("refused", [])}
    check(got_bad == refused_want, f"{what}: refused as opendmd-gallery refuses, sentence for sentence "
          f"{ {k: v for k, v in got_bad.items() if refused_want.get(k) != v} or sorted(got_bad)}")
    st, h, body = get(base, "/formulas/")
    html = body.decode()
    out = tmp / "built"
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir()
    # The same dates: the examples' as built, the contributions' as the store has them (when it first took each).
    ex_added = {e["slug"]: e["added"] for e in json.loads((site / "formulas/gallery.json").read_text())["examples"]}
    co_added = {s: e["added"] for s, e in stored.items() if e.get("added") is not None}
    want_body = gallery_page.build(out, site / "formulas/src", shown, tool, ex_added, co_added)
    got, want = lists_of(html), lists_of(gallery_page.marked(want_body))
    check(st == 200 and h.get("X-Gallery") == "store" and got == want and got[0] is not None,
          f"{what}: /formulas/ from the store (X-Gallery {h.get('X-Gallery')}), its lists as gallery_page.py builds them, "
          f"newest first, their sort and filter marks too ({'same' if got == want else 'differ'})")
    check(all(isinstance(e.get("added"), int) for e in stored.values()), f"{what}: every contribution in the store has its date")
    check(h.get("Cache-Control") == "public, max-age=60" and len(csp_of(h)) == 1 and "script-src 'self' 'wasm-unsafe-eval';" in csp_of(h)[0],
          f"{what}: /formulas/ cached 60 s, the gallery's CSP ({h.get('Cache-Control')})")
    pics = []
    for slug in sorted(set(good) & set(stored)):
        v = re.search(rf'thumbs/{slug}\.png\?v=([0-9a-f]{{12}})', html)
        st, h, body = get(base, f"/formulas/thumbs/{slug}.png" + (f"?v={v.group(1)}" if v else ""))
        try:
            same = st == 200 and png_pixels(body) == png_pixels((out / "thumbs" / f"{slug}.png").read_bytes())
        except (AssertionError, zlib.error, OSError):
            same = False
        # (a contribution identical to an example is stored, not listed: no version query to follow)
        listed = f"thumbs/{slug}.png" in (got[0] or "")
        if not (same and h.get("Content-Type") == "image/png" and (v and stored[slug]["sha"].startswith(v.group(1)) if listed else not v)):
            pics.append(f"{slug} ({st})")
            (ROOT / "target/site-worker-test").mkdir(parents=True, exist_ok=True)
            (ROOT / "target/site-worker-test" / f"{slug}.worker.png").write_bytes(body)
            shutil.copyfile(out / "thumbs" / f"{slug}.png", ROOT / "target/site-worker-test" / f"{slug}.native.png")
    check(not pics, f"{what}: every contribution's thumbnail from the store, the same pixels as opendmd-gallery's {pics}")
    srcs = []
    for slug in sorted(set(good) & set(stored)):
        st, h, body = get(base, f"/formulas/src/{slug}.psi")
        if not (st == 200 and body == (shown / f"{slug}.psi").read_bytes() and h.get("X-Gallery") == "store"):
            srcs.append(f"{slug} ({st})")
    check(not srcs, f"{what}: /formulas/src/<slug>.psi of each contribution from the store {srcs}")
    return d


def examples_from_store(base: str, site: Path) -> None:
    built = json.loads((site / "formulas/gallery.json").read_text())["examples"]
    d = gallery_api(base)
    ex = {e["slug"]: e for e in d.get("entries", []) if e["source"] in ("bundle", "example")}
    bad = [e["slug"] for e in built if e["slug"] not in ex or ex[e["slug"]]["source"] != ("bundle" if e["bundle"] else "example")
           or ex[e["slug"]]["src"] != e["src"] or ex[e["slug"]]["esp32_us"] != e["us"]]
    check(d.get("examples_build") == "this" and len(ex) == len(built) and not bad,
          f"the store holds this build's {len(built)} examples (bundled ones as 'bundle') {bad[:4]}")
    undated = [e["slug"] for e in built if ex.get(e["slug"], {}).get("added") is None
               or gallery_page.iso(ex[e["slug"]]["added"]) != e["added"]]
    check(not undated, f"every example's date in the store is the build's (opendmd-web/formulas/added.json) {undated[:4]}")
    wrong = []
    for e in built:
        st, h, body = get(base, f"/formulas/src/{e['slug']}.psi")
        if st != 200 or body != (site / "formulas/src" / f"{e['slug']}.psi").read_bytes() or h.get("X-Gallery") != "store":
            wrong.append(f"{e['slug']} {st} {h.get('X-Gallery')}")
    check(not wrong, f"/formulas/src/<stem>.psi of every example from the store, as built (/f/ and the panel read it) {wrong[:4]}")


def wait_synced(base: str, commit: str, timeout: float = 30) -> dict:
    end = time.time() + timeout
    while time.time() < end:
        d = gallery_api(base)
        if d.get("commit") == commit:
            return d
        time.sleep(0.5)
    return gallery_api(base)


def gallery_local(base: str, site: Path, tool: Path, repo: FixtureRepo) -> None:
    ex = json.loads((site / "formulas/gallery.json").read_text())["examples"]
    some = next(e for e in ex if not e["bundle"] and len(e["file"]) < 3500)
    ripple = (HEADER + 'name "Site test ripple"\nabout "Rings rippling out from the middle, for the site\'s tests"\n'
              'control speed 0..1 = 0.4 "Speed"\nloop 4\npal(rainbow, frac(len(x - 0.5, (y - 0.5)/4)*3 - t/4 + speed))\n'
              # (12 KB with its comments: the gallery's size rules are the device's, a file that compiles small is taken.)
              + '-- padding padding\n' * 600)
    glow = '-- author: Fixture & Co\n-- licence: CC-BY-4.0\n' + (
        'name "Site test glow & \'shine\'"\nabout "A light over the GIF, for the site\'s tests"\ncontrol size = 0.4 "Light size"\nlook\nloop 8\n'
        'dx = cx - 0.35*sin(t/8)*4; dy = cy - 0.3*sin(t/4)\nlayer multiply { gray(clamp(1.4 - (dx*dx + dy*dy)/(0.08 + size*0.5), 0.12, 1)) }\n')
    stars = HEADER + 'name "Site test stars"\nabout "Dots drifting, for the site\'s tests"\nloop 4\ngray(step(0.97, frac(sin(x*91 + y*37)*437 + t/4)))\n'
    broken = HEADER + 'name "Site test broken"\nabout "It does not compile"\ngray(x +)\n'
    state_a = {
        "site-test-ripple.psi": ripple,
        "site-test-glow.psi": glow,
        "copycat.psi": HEADER + some["src"],  # the same formula as an example: stored, not listed
        "site-test-broken.psi": broken,
        "site-test-reserved.psi": "-- author: 0penDMD\n-- licence: CC0-1.0\n" + ripple.split("\n", 2)[2],
        "site-test-crlf.psi": (HEADER + 'name "Site test crlf"\nabout "Windows lines"\ngray(x) -- ‮evil\n').replace("\n", "\r\n"),
        "site-test-heavy.psi": HEADER + 'name "Site test heavy"\nabout "Too heavy for the panel"\nloop 4\n' + "".join(
            f"v{k} = atan2(sin(x*{k % 9 + 1} + y*{k % 7 + 2} + t/4), {'v' + str(k - 1) if k else 'y'} + 2)\n" for k in range(20)) + "gray(v19)\n",
        "Bad_Name.psi": ripple,
        # A file over 20 KB is refused (the 12 KB ripple is taken).
        "site-test-huge.psi": ripple + "-- padding padding\n" * 1200,
        # Over 4,500 B compiled (a file of 7.7 KB) is refused; one the VM draws in 40 ms and the JIT in 7.4, compiled 4.1 KB, is taken (the tier is the gate).
        "site-test-long.psi": (ROOT / "tools/fixtures/gallery-over-4500.psi").read_text(),
        "site-test-jit.psi": (ROOT / "tools/fixtures/gallery-jit-only.psi").read_text(),
        "site-test-link.psi": ("link", "site-test-ripple.psi"),
        "site-test-dir.psi": ("dir",),
        "site-test-exec.psi": ("exec", stars),
        "notes.txt": "not a formula\n",
    }
    tmp = Path(tempfile.mkdtemp(prefix="s6-gallery-"))
    try:
        # Before anything is synced: the page as built.
        st, h, body = get(base, "/formulas/")
        check(st == 200 and h.get("X-Gallery") == "built", f"before a sync: /formulas/ as built (X-Gallery {h.get('X-Gallery')})")
        st, h, body = get(base, "/api/gallery")
        d = json.loads(body) if st == 200 else {}
        check(st == 200 and d.get("source") == repo.url and d.get("commit") is None and h.get("Access-Control-Allow-Origin") == "*",
              f"/api/gallery before a sync: no commit, the fixture repo as the source ({st} {d.get('source')})")

        # A cron run (simulated) syncs state A.
        sha_a = repo.set(state_a)
        write_state(state_a, tmp / "a")
        st, _, _ = get(base, "/cdn-cgi/handler/scheduled?cron=*/5+*+*+*+*")
        d = wait_synced(base, sha_a)
        # The sync's claim (sync_started, 10 s apart) came before its commit
        # landed, so it is no later than now: the next POST waits from here.
        # (Not from the cron request: the scheduled handler syncs in
        # waitUntil, after its reply, as late as a loaded machine makes it.)
        t0 = time.time()
        check(st == 200 and d.get("commit") == sha_a, f"a simulated cron run syncs the fixture's main ({st}, {str(d.get('commit'))[:12]})")
        examples_from_store(base, site)
        good, bad = native(tool, tmp / "a")
        want_refused = {**bad, "site-test-link": ["not a regular file (a link or a folder)"],
                        "site-test-dir": ["not a regular file (a link or a folder)"],
                        "site-test-exec": ["the file is executable: a gallery file is a plain text file (mode 100644)"]}
        check({"site-test-broken", "site-test-reserved", "site-test-crlf", "site-test-heavy", "site-test-huge", "site-test-long", "Bad_Name"} <= set(bad) and set(good) == {
            "site-test-ripple", "site-test-glow", "site-test-jit", "copycat"}, f"the fixture: opendmd-gallery passes 4 (the 12 KB one and the JIT-only one among them) and refuses the bad ones ({sorted(good)}, {sorted(bad)})")
        shown_a = tmp / "a-shown"
        write_state({k: v for k, v in state_a.items() if k[:-4] in good}, shown_a)
        before = compare_store(base, site, tool, shown_a, want_refused, "cron run", tmp)
        took = {e["slug"]: e.get("added") for e in before.get("entries", []) if e["source"] == "gallery"}
        check(took and all(isinstance(v, int) and t0 - 5 <= v <= time.time() + 5 for v in took.values()),
              f"each contribution dated when the sync first took it ({took})")
        html = get(base, "/formulas/")[2].decode()
        check(some["name"] not in lists_of(html)[0] and "Site test glow &amp; &#x27;shine&#x27;" in html and "by Fixture &amp; Co" in html,
              "the page: a contribution identical to an example left out; names and authors escaped")

        # State B by POST /api/gallery/sync: ripple broken in place, glow gone, stars new.
        state_b = {**state_a, "site-test-ripple.psi": ripple.replace("speed))", "speed)"), "site-test-stars.psi": stars}
        del state_b["site-test-glow.psi"]
        sha_b = repo.set(state_b)
        write_state(state_b, tmp / "b")
        time.sleep(max(0.0, 11 - (time.time() - t0)))
        st, h, raw = get(base, "/api/gallery/sync", "POST")
        rep = json.loads(raw) if raw else {}
        check(st == 200 and rep.get("ok") and rep.get("commit") == sha_b and rep.get("previous") == sha_a and rep.get("added") == ["site-test-stars"]
              and rep.get("gone") == ["site-test-glow"] and [x["slug"] for x in rep.get("refused", [])] == ["site-test-ripple"],
              f"POST /api/gallery/sync: 200, the new commit, stars added, glow gone, ripple's new version refused "
              f"({st} {json.dumps({k: rep.get(k) for k in ('added', 'updated', 'gone', 'kept')})})")
        st, h, raw = get(base, "/api/gallery/sync", "POST")
        check(st == 429 and h.get("Retry-After") == "10", f"POST /api/gallery/sync again at once: 429, Retry-After ({st})")
        good_b, bad_b = native(tool, tmp / "b")
        shown_b = tmp / "b-shown"
        write_state({"site-test-ripple.psi": ripple, "copycat.psi": state_a["copycat.psi"], "site-test-jit.psi": state_a["site-test-jit.psi"], "site-test-stars.psi": stars}, shown_b)
        want_b = {k: v for k, v in want_refused.items()}
        want_b["site-test-ripple"] = bad_b["site-test-ripple"]
        d = compare_store(base, site, tool, shown_b, want_b, "after a broken update", tmp)
        rip = {e["slug"]: e for e in d.get("entries", [])}.get("site-test-ripple", {})
        old = {e["slug"]: e for e in before.get("entries", [])}.get("site-test-ripple", {})
        check(rip and rip.get("sha") == old.get("sha") and rip.get("src") == old.get("src") and rip.get("added") == old.get("added"),
              "a contribution broken in place: its last good version still served, its date kept")
        dates = {e["slug"]: e.get("added") for e in d.get("entries", []) if e["source"] == "gallery"}
        first = re.findall(r'href="\.\./f/#gallery=([a-z0-9-]+)"', lists_of(get(base, "/formulas/")[2].decode())[0] or "")
        check(dates.get("site-test-stars", 0) > dates.get("site-test-ripple", 0) and first[:1] == ["site-test-stars"],
              f"the newest contribution first on the page ({first[:3]})")

        # State B2: ripple fixed with a new version: updated in place, its first date kept.
        state_b2 = {**state_b, "site-test-ripple.psi": ripple.replace("t/4 + speed", "t/2 + speed")}
        sha_b2 = repo.set(state_b2)
        time.sleep(11)
        st, h, raw = get(base, "/api/gallery/sync", "POST")
        rep = json.loads(raw) if raw else {}
        rip2 = {e["slug"]: e for e in gallery_api(base).get("entries", [])}.get("site-test-ripple", {})
        check(st == 200 and rep.get("commit") == sha_b2 and rep.get("updated") == ["site-test-ripple"] and rip2.get("sha") != old.get("sha")
              and rip2.get("added") == old.get("added") and rip2.get("updated", 0) > rip2.get("added", 0),
              f"a contribution changed: the new version served, the date it was first taken kept ({st} {rep.get('updated')})")
        write_state({**{k: state_b2[k] for k in ("site-test-ripple.psi", "copycat.psi", "site-test-jit.psi", "site-test-stars.psi")}}, shown_b)
        compare_store(base, site, tool, shown_b, {k: v for k, v in want_b.items() if k != "site-test-ripple"}, "after a good update", tmp)
        gone = [get(base, p)[0] for p in ("/formulas/src/site-test-glow.psi", "/formulas/thumbs/site-test-glow.png")]
        check(gone == [404, 404] and "Site test glow" not in get(base, "/formulas/")[2].decode(),
              f"a contribution removed: gone from the page, src/ and thumbs/ ({gone})")

        # The fixture emptied: every contribution gone, the page as built without them.
        sha_c = repo.set({})
        time.sleep(11)
        st, h, raw = get(base, "/api/gallery/sync", "POST")
        rep = json.loads(raw) if raw else {}
        check(st == 200 and rep.get("commit") == sha_c and sorted(rep.get("gone", [])) == ["copycat", "site-test-jit", "site-test-ripple", "site-test-stars"]
              and not gallery_api(base).get("refused"), f"the fixture emptied: every contribution gone, no refusals left ({st} {rep.get('gone')})")
        empty = tmp / "empty"
        write_state({}, empty)
        compare_store(base, site, tool, empty, {}, "emptied", tmp)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def github_head() -> str:
    st, h, body = get("https://github.com", "/opendmd/opendmd.github.io.git/info/refs?service=git-upload-pack")
    m = re.search(rb"([0-9a-f]{40}) refs/heads/main", body)
    return m.group(1).decode() if st == 200 and m else ""


def gallery_live(base: str, site: Path, tool: Path) -> None:
    for _ in range(3):
        st, h, raw = get(base, "/api/gallery/sync", "POST")
        if st != 429:
            break
        time.sleep(11)
    rep = json.loads(raw) if raw else {}
    head = github_head()
    check(st == 200 and rep.get("ok") and rep.get("source") == REPO and head and rep.get("commit") == head,
          f"POST /api/gallery/sync: 200, the public repo's main as GitHub says it ({st} {str(rep.get('commit'))[:12]} vs {head[:12]}, {rep.get('error')})")
    examples_from_store(base, site)
    tmp = Path(tempfile.mkdtemp(prefix="s6-gallery-live-"))
    try:
        st, h, gz = get("https://codeload.github.com", f"/opendmd/opendmd.github.io/tar.gz/{head}")
        files = {}
        with tarfile.open(fileobj=io.BytesIO(gz), mode="r:gz") as t:
            for m in t.getmembers():
                parts = m.name.split("/")
                if len(parts) == 3 and parts[1] == "gallery" and parts[2].endswith(".psi") and m.isreg() and not m.mode & 0o111:
                    files[parts[2]] = t.extractfile(m).read().decode("utf-8", "replace")
        live = tmp / "live"
        write_state(files, live)
        good, bad = native(tool, live)
        shown = tmp / "shown"
        write_state({k: v for k, v in files.items() if k[:-4] in good}, shown)
        compare_store(base, site, tool, shown, {k: v for k, v in bad.items()}, f"live ({len(files)} files in gallery/)", tmp)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)



def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("site", type=Path, help="make site's output, the folder cf dev serves")
    ap.add_argument("--base", help="test this server instead of starting cf dev")
    ap.add_argument("--port", type=int, default=0, help="cf dev's port (default: a free one)")
    ap.add_argument("--smoke", action="store_true", help="then run tools/site-smoke.py --base against it")
    ap.add_argument("--dist", type=Path, default=ROOT / "dist")
    ap.add_argument("--shots", type=Path, default=ROOT / "target/site-worker-test",
                    help="where the live panels' screenshots go")
    ap.add_argument("--art", type=Path, default=ROOT / "target/site-art")
    ap.add_argument("--gallery-tool", type=Path, default=ROOT / "opendmd-gallery/target/release/opendmd-gallery",
                    help="opendmd-gallery, built (make gallery-tool): the store is compared with it")
    a = ap.parse_args()
    if not (a.site / "_headers").is_file():
        raise SystemExit(f"{a.site} has no _headers: make site first")
    if not a.gallery_tool.is_file():
        raise SystemExit(f"no {a.gallery_tool}: make gallery-tool")
    proc = None
    repo = None
    if a.base:
        base = a.base.rstrip("/")
    else:
        port = a.port or free_port()
        repo = FixtureRepo()
        proc = start_dev(a.site, port, repo.url)
        base = f"http://127.0.0.1:{port}"
    smoke = 0
    try:
        print("the Worker's routes")
        api(base)
        formula(base)
        live = urllib.parse.urlsplit(base).hostname not in ("127.0.0.1", "localhost")
        print(f"the gallery store ({'live: the public repo' if live else 'local: a fixture repo'})")
        if repo:
            gallery_local(base, a.site, a.gallery_tool, repo)
        else:
            gallery_live(base, a.site, a.gallery_tool)
        print("pages, redirects, headers")
        pages(base, a.site)
        redirects(base, a.site)
        headers(base, a.site)
        print("CSP in the browser")
        csp_in_browser(base)
        print("live Formulas: the home page's panels, the gallery's tiles")
        live_panels(base, a.shots)
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
