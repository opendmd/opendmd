# /// script
# requires-python = ">=3.11"
# dependencies = ["markdown>=3.6"]
# ///
"""Build the public site (opendmd.github.io) as static files.

    uv run tools/site.py OUT_DIR --www opendmd-web/www --card target/demo-card

- /            the README, as the landing page
- /docs/       the user and reference docs, plus the experiments behind them
- /demo/       the web app with the demo card (tools/demo-card.py) beside it:
               with no player answering, it loads demo/index.json and plays
               that card in the browser (js/source.js, demoCard)

Links between published pages are rewritten to the HTML pages; a link to
anything not published (CLAUDE.md, source files) keeps its text and loses
the link. `make site` runs this; `make site-publish` pushes it.
"""

import argparse
import html
import posixpath
import re
import shutil
from pathlib import Path

import markdown

ROOT = Path(__file__).resolve().parent.parent

# Source (repo path) -> published path, in the order of the docs menu.
PAGES = {
    "README.md": "index.html",
    "docs/web-app.md": "docs/web-app.html",
    "docs/api.md": "docs/api.html",
    "docs/firmware.md": "docs/firmware.html",
    "docs/card.md": "docs/card.html",
    "docs/simulator.md": "docs/simulator.html",
    "docs/deploying.md": "docs/deploying.html",
    "docs/ota.md": "docs/ota.html",
    "hardware/PINOUT.md": "docs/pinout.html",
    "docs/cache-format.md": "docs/cache-format.html",
    "docs/www-pack.md": "docs/www-pack.html",
    "docs/gifslim-experiment.md": "docs/gifslim-experiment.html",
    "docs/tweening-experiment.md": "docs/tweening-experiment.html",
    "docs/art-pipeline.md": "docs/art-pipeline.html",
}
MENU = [
    ("Overview", "index.html"),
    ("Try the demo", "demo/"),
    (None, "Using it"),
    ("The web app", "docs/web-app.html"),
    ("Deploying", "docs/deploying.html"),
    ("Firmware", "docs/firmware.html"),
    ("Updates over WiFi", "docs/ota.html"),
    ("The card", "docs/card.html"),
    ("The simulator", "docs/simulator.html"),
    ("Hardware pinout", "docs/pinout.html"),
    (None, "Reference"),
    ("HTTP API", "docs/api.html"),
    ("Prepared copy format", "docs/cache-format.html"),
    ("Web app pack format", "docs/www-pack.html"),
    (None, "Background"),
    ("The GIF decoder experiment", "docs/gifslim-experiment.html"),
    ("Smooth motion experiment", "docs/tweening-experiment.html"),
    ("Art pipeline", "docs/art-pipeline.html"),
]
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
.cta{display:inline-block;margin:8px 0 24px;padding:10px 18px;border-radius:8px;background:var(--acc);color:#111;font-weight:700;text-decoration:none}
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
<link rel="stylesheet" href="{up}site.css">
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


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("out", type=Path)
    ap.add_argument("--www", type=Path, required=True)
    ap.add_argument("--card", type=Path, required=True)
    a = ap.parse_args()
    out: Path = a.out
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)

    # The demo: the app (the player sends the .gz copies; Pages compresses by itself) and its card.
    shutil.copytree(a.www, out / "demo", ignore=shutil.ignore_patterns("*.gz"))
    shutil.copytree(a.card, out / "demo/demo")

    assets: set[str] = set()
    for src, page in PAGES.items():
        text = rewrite(src, page, (ROOT / src).read_text(), assets)
        md = markdown.Markdown(extensions=["tables", "fenced_code", "toc", "sane_lists"],
                               extension_configs={"toc": {"permalink": "#"}})
        body = md.convert(text)
        if page == "index.html":
            # The landing page: the demo first, under the intro.
            at = body.find("</p>", body.find("hero.webp")) + 4
            body = body[:at] + '\n<p><a class="cta" href="demo/">Try the demo in your browser →</a></p>' + body[at:]
        m = re.search(r"<h1[^>]*>(.*?)</h1>", body, re.S)
        title = re.sub(r"<[^>]+>", "", m.group(1)).strip() if m else "OpenDMD"
        title = "OpenDMD" if title == "OpenDMD" else f"{title} · OpenDMD"
        up = "../" * page.count("/")
        dest = out / page
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text(PAGE.format(title=html.escape(title), up=up, menu=menu(page), body=body))
    for target in sorted(assets):
        (out / target).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / target, out / target)
    for src, dst in FILES.items():
        shutil.copyfile(ROOT / src, out / dst)
    (out / "site.css").write_text(CSS.strip() + "\n")
    (out / ".nojekyll").write_text("")
    n = sum(1 for p in out.rglob("*") if p.is_file())
    size = sum(p.stat().st_size for p in out.rglob("*") if p.is_file())
    print(f"{out}: {n} files, {size / 1e6:.1f} MB")


if __name__ == "__main__":
    main()
