# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright>=1.45"]
# ///
"""The opendmd-site Worker (site/worker/) over HTTP, against `cf dev`
serving a built site (make site):

    uv run tools/site-worker-test.py target/site [--base URL] [--smoke]

Without --base it starts `cf dev` itself (site/worker's pinned cf and
wrangler: make site-worker-deps) on a free port, and stops it after. With
--smoke, tools/site-smoke.py then runs against the same server (--base).

- /api/health: 200 JSON {ok, site, version}, CORS *, not cached; HEAD,
  OPTIONS 204, POST 405 with Allow; another /api/ path 404 JSON;
- /f/ (the static Formula page) served through the Worker with its CORS and
  its CSP; a short-link-shaped /f/<id> answers the 404 page, never cached;
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


def get(base: str, path: str, method: str = "GET", headers: dict | None = None):
    """(status, headers, body) without following redirects."""
    req = urllib.request.Request(base + path, method=method, headers={"User-Agent": UA, **(headers or {})})
    try:
        with OPENER.open(req, timeout=30) as r:
            return r.status, r.headers, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.headers, e.read()


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
    st, h, body = get(base, "/api/f")
    check(st == 404 and json.loads(body) == {"error": "not found"}, f"/api/f (not yet): 404 JSON ({st})")


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
          f"/f/<id>: the 404 page, never cached ({st}, {h.get('Cache-Control')})")
    st, h, body = get(base, "/f/no-such-file.js")
    check(st == 404 and h.get("Cache-Control") != "no-store", f"/f/<not an id>: a plain 404 ({st}, {h.get('Cache-Control')})")
    www(base)


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


def csp_in_browser(base: str) -> None:
    from playwright.sync_api import sync_playwright
    paths = ["/", "/guide/web-app", "/install/", "/formulas/", "/licenses/", "/dev/api", "/no/such/page", "/demo/",
             "/f/#" + urllib.parse.urlencode({"formula": "loop 4\npal(rainbow, frac(x + t/4))", "name": "Rainbow"})]
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
        print("pages, redirects, headers")
        pages(base, a.site)
        redirects(base, a.site)
        headers(base, a.site)
        print("CSP in the browser")
        csp_in_browser(base)
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
