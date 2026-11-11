#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.10"
# dependencies = ["adblock>=0.6"]
# ///
"""No request URL the panel's web app or the site makes may match an ad blocker's list.

uBlock Origin with EasyPrivacy blocked `/api/stat?` (filter `/api/stat?`,
type xhr, first-party) and the Formulas view said "NetworkError". A path
that looks like analytics ("stat", "track", "log", "count", "beacon", ...)
is blocked for a real share of users, and the failure looks like the panel
being down. This checks every URL against the two lists most people run:

  EasyPrivacy  https://easylist.to/easylist/easyprivacy.txt
  EasyList     https://easylist.to/easylist/easylist.txt

The matcher is `adblock` (Brave's adblock-rust engine, the one in Brave's
shields, with Python bindings): real ABP syntax -- $xmlhttprequest,
$third-party, $domain=, ^ separators, anchors, regex filters, exceptions
(@@) and $important -- not a regex approximation.

Where the URLs come from (so a new route is covered without editing this):
  - the panel's routes: every server.route / route_json in firmware/src/http.rs
    (wildcards filled with sample names), with the query strings the web
    app's source and tools/mock-device.py use;
  - the web app's assets: every file under opendmd-web/www and the wasm
    packages' files (pkg, pkg-art, pkg-psi, pkg-studio);
  - the site's Worker routes: site/worker/src/index.ts (and its /formulas/
    paths), the site's pages: the built site if there is one (--site, default
    target/site), else site/pages, the theme and public-repo sources.

Each is asked as the web app asks (xhr for the API, script/stylesheet/font/
image/document for assets), first-party from http://opendmd.local/ and from a
LAN IP, and the site's from https://opendmd.dev/ (the Worker's routes also as
the panel's app calls them: third-party from the panel).

The lists are cached under target/blocklists/ with the date; it fetches when
today's copy is missing, uses the newest cache when offline (--offline never
fetches), and fails clearly with no cache at all.

Exit 0: nothing matched. Exit 1: matches, printed with the filter and list.
Exit 2: no lists.
"""
import argparse
import datetime
import json
import re
import sys
import urllib.request
from pathlib import Path

import adblock

ROOT = Path(__file__).resolve().parent.parent
LISTS = {
    "EasyPrivacy": "https://easylist.to/easylist/easyprivacy.txt",
    "EasyList": "https://easylist.to/easylist/easylist.txt",
}
PANEL_HOSTS = ["http://opendmd.local", "http://192.168.1.50"]
SITE = "https://opendmd.dev"
# What a wildcard route stands for, and what the web app puts in a query.
SAMPLE = {"*": "Sample%20Name.gif"}
Q_SAMPLE = "%2Fformulas"
EXTRA_QUERIES = ["?path=%2Fformulas", "?dir=%2Fgifs&sizes=1", "?dir=%2F", "?name=opendmd", "?names=1&dir=%2Fgifs"]


def load_lists(cache_dir, offline):
    cache_dir.mkdir(parents=True, exist_ok=True)
    today = datetime.date.today().isoformat()
    out = {}
    for name, url in LISTS.items():
        slug = name.lower()
        today_file = cache_dir / f"{slug}-{today}.txt"
        if not today_file.exists() and not offline:
            try:
                req = urllib.request.Request(url, headers={"User-Agent": "opendmd-blocklist-check"})
                data = urllib.request.urlopen(req, timeout=60).read()
                if len(data) < 10000 or b"[Adblock" not in data[:200]:
                    raise ValueError(f"not an Adblock list ({len(data)} bytes)")
                today_file.write_bytes(data)
                print(f"fetched {name} -> {today_file.relative_to(ROOT)} ({len(data) // 1024} KB)")
            except Exception as e:  # noqa: BLE001
                print(f"could not fetch {name}: {e}", file=sys.stderr)
        files = sorted(cache_dir.glob(f"{slug}-*.txt"))
        if not files:
            print(f"error: no cached {name} in {cache_dir} and it could not be fetched; "
                  f"run once with network (or save {url} as {today_file.name} there)", file=sys.stderr)
            sys.exit(2)
        out[name] = files[-1]
    return out


def engines(files):
    res = {}
    for name, path in files.items():
        fs = adblock.FilterSet(debug=True)
        fs.add_filter_list(path.read_text(errors="replace"))
        res[name] = adblock.Engine(fs)
        print(f"{name}: {path.name}")
    return res


def sub_wild(p):
    return p.replace("/*", "/" + SAMPLE["*"]).replace("*", "")


def firmware_routes():
    src = (ROOT / "firmware/src/http.rs").read_text()
    paths = set()
    for m in re.finditer(r'\.route(?:_json)?\(\s*"(/[^"]*)"', src):
        paths.add(m.group(1))
    return sorted(paths)


def js_queries():
    """`/api/x?a=` literals in the web app and the tools: path -> {"?a=", ...}."""
    q = {}
    files = list((ROOT / "opendmd-web/www").rglob("*.js")) + [ROOT / "tools/mock-device.py"]
    for f in files:
        if "pkg" in f.parts[-3:-1] or f.name.endswith(".min.js"):
            continue
        for m in re.finditer(r"""['"`](/api/[A-Za-z0-9_/.-]+)\?([A-Za-z0-9_=&${}.%-]*)""", f.read_text(errors="replace")):
            qs = re.sub(r"\$\{[^}]*\}", Q_SAMPLE, m.group(2))
            qs = re.sub(r"=(?=$|&)", "=" + Q_SAMPLE, qs)
            q.setdefault(m.group(1), set()).add("?" + qs)
    return q


def panel_urls():
    """(path+query, type) a panel's web app requests."""
    queries = js_queries()
    out = []
    for p in firmware_routes():
        base = sub_wild(p)
        typ = "xmlhttprequest" if p.startswith("/api/") else "other"
        if p == "/" or p == "/setup":
            typ = "document"
        out.append((base, typ))
        if p.startswith("/api/"):
            for qs in sorted(queries.get(p, set()) | set(EXTRA_QUERIES)):
                out.append((base + qs, typ))
    return out


def web_assets():
    www = ROOT / "opendmd-web/www"
    rels = set()
    for f in www.rglob("*"):
        if f.is_file() and not f.name.endswith(".gz"):
            rels.add(f.relative_to(www).as_posix())
    # The wasm packages are built, not committed: their file names are the crates'.
    for d, crate in (("pkg", "opendmd_web"), ("pkg-art", "opendmd_art_web"), ("pkg-psi", "opendmd_psi_web"), ("pkg-studio", "opendmd_studio_web")):
        crate_name = crate
        toml = {"pkg": "opendmd-web", "pkg-art": "opendmd-art-web", "pkg-psi": "opendmd-psi-web", "pkg-studio": "opendmd-studio-web"}[d]
        ct = ROOT / toml / "Cargo.toml"
        if ct.exists():
            m = re.search(r'^name\s*=\s*"([^"]+)"', ct.read_text(), re.M)
            if m:
                crate_name = m.group(1).replace("-", "_")
        rels.add(f"{d}/{crate_name}.js")
        rels.add(f"{d}/{crate_name}_bg.wasm")
    rels |= {"version.txt", "formulas.json", "licenses.txt"}
    return sorted(rels)


def asset_type(rel):
    ext = rel.rsplit(".", 1)[-1].lower() if "." in rel else ""
    return {"js": "script", "mjs": "script", "css": "stylesheet", "woff2": "font", "woff": "font",
            "svg": "image", "png": "image", "gif": "image", "ico": "image", "html": "document",
            "wasm": "xmlhttprequest", "json": "xmlhttprequest", "txt": "xmlhttprequest"}.get(ext, "other")


def worker_paths():
    src = (ROOT / "site/worker/src/index.ts").read_text()
    paths = set(re.findall(r'pathname\s*===\s*"(/[^"]*)"', src))
    paths |= {"/formulas/", "/formulas/src/sample.psi", "/formulas/thumbs/sample.png"}
    for m in re.finditer(r'startsWith\("(/[^"]*)"\)', src):
        if m.group(1) not in ("/api/",):
            paths.add(m.group(1))
    return sorted(paths)


def site_paths(site_dir):
    paths = set()
    if site_dir.is_dir():
        for f in site_dir.rglob("*"):
            if f.is_file() and not f.name.endswith((".gz", ".br")):
                rel = f.relative_to(site_dir).as_posix()
                paths.add("/" + rel)
                if rel.endswith("/index.html"):
                    paths.add("/" + rel[: -len("index.html")])
        return sorted(paths), f"built site {site_dir}"
    paths.add("/")
    for sub in ("site/pages", "site/theme", "site/public-repo"):
        for f in (ROOT / sub).rglob("*"):
            if f.is_file():
                rel = f.relative_to(ROOT / sub).as_posix()
                paths.add("/" + (rel[:-3] + "/" if rel.endswith(".md") else rel))
    paths |= {"/guide/", "/install/", "/create/", "/studio/", "/demo/", "/dev/api/", "/formulas/", "/install/release.json", "/install/beta.json", "/art-index/v1/NES.json"}
    return sorted(paths), "site sources (no built site; run make site for the full file list)"


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--offline", action="store_true", help="never fetch: use the newest cached lists")
    ap.add_argument("--cache", default=str(ROOT / "target/blocklists"))
    ap.add_argument("--site", default=str(ROOT / "target/site"), help="a built site (make site) to take its files from")
    ap.add_argument("--list-urls", action="store_true", help="print the URLs checked and exit")
    a = ap.parse_args()

    reqs = []  # (url, source, type, label)
    for host in PANEL_HOSTS:
        src = host + "/"
        for p, t in panel_urls():
            reqs.append((host + p, src, t, f"panel route from {host}"))
        for rel in web_assets():
            reqs.append((f"{host}/{rel}", src, asset_type(rel), f"web asset from {host}"))
    sp, how = site_paths(Path(a.site))
    for p in worker_paths():
        t = "xmlhttprequest" if p.startswith("/api/") else "other"
        reqs.append((SITE + p, SITE + "/", t, "worker route, first-party"))
        reqs.append((SITE + p, PANEL_HOSTS[0] + "/", t, "worker route, third-party from the panel's app"))
    for p in sp:
        t = asset_type(p.rsplit("/", 1)[-1]) if "." in p.rsplit("/", 1)[-1] else "document"
        reqs.append((SITE + p, SITE + "/", t, "site page/asset"))
        if p.endswith((".json", ".psi", ".txt")):
            reqs.append((SITE + p, PANEL_HOSTS[0] + "/", "xmlhttprequest", "site file, third-party from the panel's app"))

    # The counted requests (site/worker/src/counts.ts) as they are made, query
    # strings included: the web app's update check (update.js, from the panel's
    # page and from the demo) and the installer's firmware download (ESP Web
    # Tools' fetch, from /install/), and the page that shows the counts.
    rel_ver = json.loads(Path(a.site, "install/release.json").read_text())["version"] if Path(a.site, "install/release.json").is_file() else "1.0.0-beta.14"
    for host in PANEL_HOSTS + [SITE + "/demo"]:
        for lay in ("4mb",):
            for feed in ("release.json", "beta.json"):  # beta.json: Get beta updates on
                reqs.append((f"{SITE}/install/{feed}?v={rel_ver}&layout={lay}", host + "/", "xmlhttprequest", "counted: the update check"))
    for lay in ("4mb",):
        for part in ("app", "full"):
            for d in ("firmware", "firmware/beta"):  # firmware/beta/: a beta's installer, after a final release
                reqs.append((f"{SITE}/install/{d}/opendmd-{rel_ver}-{lay}-{part}.bin", SITE + "/install/", "xmlhttprequest", "counted: an install"))
    for p in ("/stats", "/stats/"):
        reqs.append((SITE + p, SITE + "/", "document", "the usage page"))

    if a.list_urls:
        for r in reqs:
            print(r[2], r[0], "<-", r[1])
        return 0

    eng = engines(load_lists(Path(a.cache), a.offline))
    print(f"routes: {len(firmware_routes())} firmware, {len(worker_paths())} worker; site: {how}")
    seen = set()
    blocked = []
    for url, src, typ, label in reqs:
        if (url, src, typ) in seen:
            continue
        seen.add((url, src, typ))
        for lname, e in eng.items():
            r = e.check_network_urls(url, src, typ)
            if r.matched and not r.exception:
                blocked.append((url, typ, label, lname, r.filter))
    print(f"checked {len(seen)} requests against {len(eng)} lists")
    if not blocked:
        print("blocklist-check: PASS, no URL matches EasyPrivacy or EasyList")
        return 0
    print(f"\nblocked ({len(blocked)}):")
    for url, typ, label, lname, flt in blocked:
        print(f"  {url}\n    type={typ} ({label})\n    {lname}: {flt}")
    print(f"\nblocklist-check: FAIL, {len({b[0] for b in blocked})} URLs would be blocked")
    return 1


if __name__ == "__main__":
    sys.exit(main())
