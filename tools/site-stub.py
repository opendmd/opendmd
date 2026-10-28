# /// script
# requires-python = ">=3.11"
# ///
"""The GitHub Pages stub: what opendmd.github.io serves once the site lives
at opendmd.dev (docs/research/site-cloudflare.md §7, phase M2).

    uv run tools/site-stub.py target/site target/site-stub [--to https://opendmd.dev]

Web apps already on players ask the old host for two things with `fetch`,
which does not follow a meta refresh, so those stay real files there:

- install/release.json (and beta.json when the site has one), the update
  check: the same JSON with `firmware`, `www` and every variant's
  `firmware` made absolute URLs on the new host (update.js resolves them
  against the JSON's own address, so an absolute URL wins). opendmd.dev's
  install/ answers CORS `*`, so the download works cross-origin;
- art-index/ (every format), Create Art's game names: copied as they are.

The installer's ESP Web Tools manifests (install/firmware/manifest*.json)
stay too, their parts' paths absolute on the new host, for an installer tab
left open across the move. Every other page of the site becomes a small
HTML page that sends the browser to the same address on the new host:
`location.replace(new + pathname + search + hash)` first, so a shared
`/f/#formula=…` link keeps its Formula (a meta refresh drops the hash),
then a meta refresh and a link for a browser without JavaScript. The 404
page does the same for any address the stub does not have (GitHub Pages
serves 404.html there). Nothing else is carried: no scripts, styles,
pictures or firmware files.

tools/site-publish.sh publishes the stub as it publishes the site (keeping
the public repo's gallery/, .github/, dev/api/ and art-index/).
tools/site-stub-test.sh checks it.
"""

import argparse
import html
import json
import re
import shutil
from pathlib import Path
from urllib.parse import urljoin

KEEP_JSON = ("install/release.json", "install/beta.json")
MANIFEST = re.compile(r"install/firmware/manifest[^/]*\.json")


def page_url(to: str, rel: str) -> str:
    """The new address of a published page: x/index.html is x/."""
    return f"{to}/{rel[:-len('index.html')] if rel == 'index.html' or rel.endswith('/index.html') else rel}"


def stub(to: str, fallback: str, canonical: bool) -> str:
    js = json.dumps(to)
    url = html.escape(fallback, quote=True)
    host = html.escape(re.sub(r"^https?://", "", to))
    return (f'<!doctype html>\n<html lang="en"><head><meta charset="utf-8">\n'
            f"<title>OpenDMD has moved to {host}</title>\n"
            f"<script>location.replace({js} + location.pathname + location.search + location.hash)</script>\n"
            f'<meta http-equiv="refresh" content="0; url={url}">\n'
            + (f'<link rel="canonical" href="{url}">\n' if canonical else "")
            + f'<meta name="robots" content="noindex">\n</head><body>\n'
            f'<p>OpenDMD\'s site is now at <a href="{url}">{host}</a>.</p>\n</body></html>\n')


def absolute(base: str, value: str) -> str:
    return urljoin(base, value)


def release_json(to: str, src: Path, dst: Path) -> None:
    info = json.loads(src.read_text())
    base = f"{to}/install/{src.name}"
    for key in ("firmware", "www"):
        if isinstance(info.get(key), str):
            info[key] = absolute(base, info[key])
    for v in (info.get("variants") or {}).values():
        if isinstance(v, dict) and isinstance(v.get("firmware"), str):
            v["firmware"] = absolute(base, v["firmware"])
    dst.write_text(json.dumps(info, indent=2) + "\n")


def manifest_json(to: str, rel: str, src: Path, dst: Path) -> None:
    m = json.loads(src.read_text())
    base = f"{to}/{rel}"
    for b in m.get("builds", []):
        for p in b.get("parts", []):
            p["path"] = absolute(base, p["path"])
    dst.write_text(json.dumps(m, indent=2) + "\n")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("site", type=Path, help="make site's output")
    ap.add_argument("out", type=Path)
    ap.add_argument("--to", default="https://opendmd.dev", help="the site's new home (no trailing slash)")
    a = ap.parse_args()
    to = a.to.rstrip("/")
    if not re.fullmatch(r"https?://[A-Za-z0-9.-]+(:\d+)?", to):
        raise SystemExit(f"--to: an origin like https://opendmd.dev, not {a.to!r}")
    if not (a.site / "install/release.json").is_file():
        raise SystemExit(f"{a.site} has no install/release.json: make site first")
    if a.out.exists():
        shutil.rmtree(a.out)
    a.out.mkdir(parents=True)
    pages = json_files = 0
    for src in sorted(p for p in a.site.rglob("*") if p.is_file()):
        rel = src.relative_to(a.site).as_posix()
        dst = a.out / rel
        if rel in KEEP_JSON:
            dst.parent.mkdir(parents=True, exist_ok=True)
            release_json(to, src, dst)
            json_files += 1
        elif MANIFEST.fullmatch(rel):
            dst.parent.mkdir(parents=True, exist_ok=True)
            manifest_json(to, rel, src, dst)
            json_files += 1
        elif rel.startswith("art-index/"):
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(src, dst)
            json_files += 1
        elif rel == "404.html":
            dst.write_text(stub(to, to + "/", canonical=False))
        elif rel.endswith(".html"):
            dst.parent.mkdir(parents=True, exist_ok=True)
            dst.write_text(stub(to, page_url(to, rel), canonical=True))
            pages += 1
    if not (a.out / "404.html").exists():
        (a.out / "404.html").write_text(stub(to, to + "/", canonical=False))
    (a.out / ".nojekyll").write_text("")
    host = re.sub(r"^https?://", "", to)
    (a.out / "README.md").write_text(
        f"# opendmd.github.io\n\nThe OpenDMD site has moved to {to}/. This repository keeps forwarding its old "
        f"addresses there (a shared Formula link keeps its Formula), the update check's install/release.json "
        f"(pointing at {host}) and Create Art's art-index/ for players whose web app still asks here, and takes "
        f"formula gallery contributions in gallery/ (see gallery/README.md). Built by `make site-stub` in the "
        f"OpenDMD repository; changes made here are overwritten by the next publish.\n")
    print(f"{a.out}: {pages} pages forwarded to {to}, {json_files} JSON files kept")


if __name__ == "__main__":
    main()
