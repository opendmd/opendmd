# /// script
# requires-python = ">=3.11"
# dependencies = ["markdown>=3.6"]
# ///
"""Build the public site (opendmd.github.io) as static files.

    uv run tools/site.py OUT_DIR --www opendmd-web/www --card target/demo-card --dist dist

- /            the README, as the landing page
- /install/    the browser installer (docs/guide/install.md with ESP Web
               Tools' button) and, under install/firmware/, the release in
               dist/: the parts dist/manifest.json names, the merged image
               and the signed update bundle, plus install/release.json
               (what the web app's update check reads)
- /guide/      the user guide (docs/guide/)
- /dev/        the developer pages: an overview and a few clean references
               (the detailed notes in docs/ stay in the repo)
- /demo/       the web app with the demo card (tools/demo-card.py) beside it:
               with no player answering, it loads demo/index.json and plays
               that card in the browser (js/source.js, demoCard)

Links between published pages are rewritten to the HTML pages; a link to
anything not published (CLAUDE.md, source files) keeps its text and loses
the link. `make site` runs this; `make site-publish` pushes it.
"""

import argparse
import html
import json
import posixpath
import re
import shutil
from pathlib import Path

import markdown

ROOT = Path(__file__).resolve().parent.parent

# Source (repo path) -> published path, in the order of the docs menu.
PAGES = {
    "README.md": "index.html",
    "docs/guide/install.md": "install/index.html",
    "docs/guide/web-app.md": "guide/web-app.html",
    "docs/guide/create-art.md": "guide/create-art.html",
    "docs/guide/effects.md": "guide/effects.html",
    "docs/guide/card.md": "guide/card.html",
    "docs/guide/updating.md": "guide/updating.html",
    "docs/guide/troubleshooting.md": "guide/troubleshooting.html",
    "docs/developers.md": "dev/index.html",
    "docs/api.md": "dev/api.html",
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
}
MENU = [
    ("Overview", "index.html"),
    ("Try the demo", "demo/"),
    (None, "Guide"),
    ("Install", "install/index.html"),
    ("The web app", "guide/web-app.html"),
    ("Create Art", "guide/create-art.html"),
    ("Effects and live items", "guide/effects.html"),
    ("The SD card", "guide/card.html"),
    ("Updating", "guide/updating.html"),
    ("Troubleshooting", "guide/troubleshooting.html"),
    (None, "For developers"),
    ("Building from source", "dev/index.html"),
    ("HTTP API", "dev/api.html"),
    ("The simulator", "dev/simulator.html"),
    ("Hardware pinout", "dev/pinout.html"),
]
# The installer page: ESP Web Tools, pinned (the flashing itself is
# esptool-js over Web Serial; Chrome or Edge, https or localhost).
INSTALL_PAGE = "install/index.html"
INSTALL_HEAD = ('<script type="module" src="https://cdn.jsdelivr.net/npm/esp-web-tools@10.4.0/dist/web/'
                'install-button.js?module"></script>')
INSTALL_BUTTON = """<div class="install">
<esp-web-install-button id="install" manifest="firmware/manifest.json">
<button slot="activate" id="install-activate">Install</button>
<span slot="unsupported">This browser can't talk to USB boards. Open this page in Chrome or Edge on a computer.</span>
<span slot="not-allowed">This page has to be opened over https.</span>
</esp-web-install-button>
<span class="ver">OpenDMD {version}</span>
</div>"""
# Other files published as they are: repo path -> published path.
FILES = {
    "LICENSE-MIT": "LICENSE-MIT.txt",
    "LICENSE-APACHE": "LICENSE-APACHE.txt",
}

CSS = """
:root{--bg:#0e0f11;--bg2:#16171a;--bg3:#1e2024;--line:#2a2c31;--fg:#e9e8e4;--mut:#9a9ca3;--acc:#f0a33a;color-scheme:dark}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--fg);font:16px/1.6 system-ui,-apple-system,"Segoe UI",sans-serif}
a{color:var(--acc)}
.wrap{display:grid;grid-template-columns:240px minmax(0,1fr);max-width:1240px;margin:0 auto}
nav{position:sticky;top:0;align-self:start;height:100vh;overflow:auto;padding:24px 16px;border-right:1px solid var(--line)}
nav .brand{display:block;font-weight:700;font-size:19px;color:var(--fg);text-decoration:none;margin:0 8px 16px}
nav a{display:block;padding:5px 8px;border-radius:6px;color:var(--mut);text-decoration:none;font-size:14px}
nav a:hover{color:var(--fg);background:var(--bg3)}
nav a.on{color:var(--fg);background:var(--bg3);font-weight:600}
nav a.demo{color:#111;background:var(--acc);font-weight:600;margin:4px 0 8px}
nav h4{margin:18px 8px 4px;font-size:12px;letter-spacing:.06em;text-transform:uppercase;color:var(--mut)}
main{padding:32px 40px 80px;min-width:0}
main>*{max-width:860px}
h1,h2,h3{line-height:1.25;text-wrap:balance}
h1{font-size:34px;margin:8px 0 16px}h2{font-size:24px;margin:40px 0 12px;padding-top:8px;border-top:1px solid var(--line)}h3{font-size:18px;margin:28px 0 8px}
h1 .headerlink{display:none}
h2 .headerlink,h3 .headerlink{opacity:0;margin-left:6px;text-decoration:none;color:var(--mut)}
h2:hover .headerlink,h3:hover .headerlink{opacity:1}
code{font:14px ui-monospace,Menlo,Consolas,monospace;background:var(--bg3);padding:1px 5px;border-radius:4px}
pre{background:var(--bg2);border:1px solid var(--line);border-radius:8px;padding:14px 16px;overflow:auto}
pre code{background:none;padding:0;font-size:13px;line-height:1.5}
table{border-collapse:collapse;display:block;overflow:auto;margin:16px 0}
th,td{border:1px solid var(--line);padding:6px 10px;text-align:left;vertical-align:top}
th{background:var(--bg2)}
img{max-width:100%;height:auto;border-radius:6px}
blockquote{margin:16px 0;padding:4px 16px;border-left:3px solid var(--acc);color:var(--mut)}
hr{border:0;border-top:1px solid var(--line)}
.install{display:flex;flex-wrap:wrap;align-items:center;gap:16px;margin:8px 0 24px;padding:20px;border:1px solid var(--line);border-radius:10px;background:var(--bg2)}
.install button{font:inherit;font-weight:700;padding:10px 26px;border-radius:8px;border:0;background:var(--acc);color:#111;cursor:pointer}
.install .ver{color:var(--mut);font:14px ui-monospace,Menlo,Consolas,monospace}
.cta{display:inline-block;margin:8px 8px 24px 0;padding:10px 18px;border-radius:8px;background:var(--acc);color:#111;font-weight:700;text-decoration:none}
.cta.alt{background:none;color:var(--acc);box-shadow:inset 0 0 0 2px var(--acc)}
@media (max-width:860px){
  .wrap{display:block}
  nav{position:static;height:auto;border-right:0;border-bottom:1px solid var(--line);padding:16px}
  nav .links{display:flex;flex-wrap:wrap;gap:4px}nav h4{display:none}
  main{padding:20px 16px 64px}
  h1{font-size:28px}
}
"""

PAGE = """<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="dark">
<title>{title}</title>
<link rel="stylesheet" href="{up}site.css">{head}
</head>
<body>
<div class="wrap">
<nav><a class="brand" href="{up}index.html">OpenDMD</a><div class="links">{menu}</div></nav>
<main>
{body}
</main>
</div>
</body>
</html>
"""

LINK = re.compile(r'(\]\(|(?:src|href)=")([^)"\s]+)')


def rel(from_page: str, to: str) -> str:
    """A link from one published page to another published path."""
    return posixpath.relpath(to, posixpath.dirname(from_page) or ".") + ("/" if to.endswith("/") else "")


def rewrite(src: str, page: str, text: str, assets: set[str]) -> str:
    def fix(m: re.Match) -> str:
        lead, url = m.groups()
        if re.match(r"[a-z]+:|#|/", url):
            return m.group(0)
        path, _, frag = url.partition("#")
        target = posixpath.normpath(posixpath.join(posixpath.dirname(src), path))
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
    names = {page: label for label, page in MENU if label}
    def title(m: re.Match) -> str:
        target = posixpath.normpath(posixpath.join(posixpath.dirname(src), m.group(2).partition("#")[0]))
        return f"[{names[PAGES[target]]}]({m.group(2)})" if PAGES.get(target) in names else m.group(0)
    text = re.sub(r"\[`([^`\]]+\.md)`\]\(([^)\s]+)\)", title, text)
    text = LINK.sub(fix, text)
    # Markdown links to unpublished files: keep the text, drop the link.
    return re.sub(r"\[([^\]]+)\]\(#unpublished:[^)]*\)", r"\1", text)


def menu(page: str) -> str:
    out = []
    for label, target in MENU:
        if label is None:
            out.append(f"<h4>{html.escape(target)}</h4>")
            continue
        cls = "demo" if target == "demo/" else "on" if target == page else ""
        out.append(f'<a href="{rel(page, target)}"{f" class={cls!r}" if cls else ""}>{html.escape(label)}</a>')
    return "".join(out)


def release(dist: Path, out: Path) -> dict:
    """The current release in dist/ (fw-release keeps older versions there
    too): the parts its manifest names, the merged image and the signed
    bundle of that version only, under install/firmware/; and
    install/release.json for the web app's update check."""
    manifest = dist / "manifest.json"
    if not manifest.is_file():
        raise SystemExit(f"no {manifest}: run make fw-release first")
    m = json.loads(manifest.read_text())
    ver = m["version"]
    fw = out / "install/firmware"
    fw.mkdir(parents=True)
    names = [p["path"] for b in m["builds"] for p in b["parts"]]
    names += [f"opendmd-{ver}-4mb-full.bin", f"opendmd-{ver}.odfw"]
    for name in ["manifest.json", *names]:
        if not (dist / name).is_file():
            raise SystemExit(f"no {dist / name}: run make fw-release")
        shutil.copyfile(dist / name, fw / name)
    # The www pack's version: the header's string (docs/www-pack.md), the
    # one GET /api/www reports.
    pack = (fw / f"opendmd-{ver}-www.pack").read_bytes()[:64]
    if pack[:4] != b"ODWP":
        raise SystemExit(f"opendmd-{ver}-www.pack is not a www pack")
    www_version = pack[20:20 + pack[18]].decode()
    info = {"version": ver, "firmware": f"firmware/opendmd-{ver}.odfw",
            "www": f"firmware/opendmd-{ver}-www.pack", "www_version": www_version}
    (out / "install/release.json").write_text(json.dumps(info, indent=2) + "\n")
    return info


def markers(page: str, text: str, rel_info: dict) -> str:
    """Site-only pieces, written in the docs as HTML comments (invisible on
    GitHub): the install button and the update bundle's download link."""
    ver = html.escape(rel_info["version"])
    bundle = rel(page, "install/" + rel_info["firmware"])
    text = text.replace("<!-- site:install-button -->", INSTALL_BUTTON.format(version=ver))
    return text.replace("<!-- site:firmware-bundle -->",
                        f'<p><a href="{bundle}" download>Download the firmware update file</a> '
                        f'(OpenDMD {ver}, <code>opendmd-{ver}.odfw</code>)</p>')


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("out", type=Path)
    ap.add_argument("--www", type=Path, required=True)
    ap.add_argument("--card", type=Path, required=True)
    ap.add_argument("--dist", type=Path, required=True, help="the release from make fw-release")
    a = ap.parse_args()
    out: Path = a.out
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)

    # The demo: the app (the player sends the .gz copies; Pages compresses by itself) and its card.
    shutil.copytree(a.www, out / "demo", ignore=shutil.ignore_patterns("*.gz", ".gitignore"))
    shutil.copytree(a.card, out / "demo/demo")
    info = release(a.dist, out)

    assets: set[str] = set()
    for src, page in PAGES.items():
        text = markers(page, rewrite(src, page, (ROOT / src).read_text(), assets), info)
        md = markdown.Markdown(extensions=["tables", "fenced_code", "toc", "sane_lists"],
                               extension_configs={"toc": {"permalink": "#"}})
        body = md.convert(text)
        if page == "index.html":
            # The landing page: the demo first, under the intro.
            at = body.find("</p>", body.find("hero.webp")) + 4
            body = body[:at] + ('\n<p><a class="cta" href="demo/">Try the demo in your browser →</a> '
                                       '<a class="cta alt" href="install/">Install on your display</a></p>') + body[at:]
        m = re.search(r"<h1[^>]*>(.*?)</h1>", body, re.S)
        title = re.sub(r"<[^>]+>", "", m.group(1)).strip() if m else "OpenDMD"
        title = "OpenDMD" if title == "OpenDMD" else f"{title} · OpenDMD"
        up = "../" * page.count("/")
        dest = out / page
        dest.parent.mkdir(parents=True, exist_ok=True)
        head = "\n" + INSTALL_HEAD if page == INSTALL_PAGE else ""
        dest.write_text(PAGE.format(title=html.escape(title), up=up, head=head, menu=menu(page), body=body))
    for target in sorted(assets):
        (out / target).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / target, out / target)
    for src, dst in FILES.items():
        shutil.copyfile(ROOT / src, out / dst)
    (out / "site.css").write_text(CSS.strip() + "\n")
    (out / ".nojekyll").write_text("")
    for src, dst in MOVED.items():
        to = html.escape(rel(src, dst))
        (out / src).parent.mkdir(parents=True, exist_ok=True)
        (out / src).write_text(f'<!doctype html><meta charset="utf-8"><title>Moved</title>'
                               f'<meta http-equiv="refresh" content="0; url={to}"><link rel="canonical" href="{to}">'
                               f'<p>This page moved: <a href="{to}">{to}</a></p>\n')
    (out / "README.md").write_text("# opendmd.github.io\n\nThe OpenDMD site: documentation and a browser demo of the web app "
                                   "(https://opendmd.github.io/). Built by `make site` in the OpenDMD repository; "
                                   "changes made here are overwritten by the next publish.\n")
    n = sum(1 for p in out.rglob("*") if p.is_file())
    size = sum(p.stat().st_size for p in out.rglob("*") if p.is_file())
    print(f"{out}: {n} files, {size / 1e6:.1f} MB")


if __name__ == "__main__":
    main()
