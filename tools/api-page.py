# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Render docs/api.md as the web app's API reference page.

    uv run tools/api-page.py [docs/api.md] [opendmd-web/www/api.html]

Run by `make web` on every build (before the gzip and the pack), so the page
never drifts from the Markdown. Stdlib only: the converter handles the subset
docs/api.md uses — `#`..`###` headings, paragraphs, `-` lists (continuation
lines indented), fenced code, pipe tables, and inline code, bold, italics and
links. Anything else is a paragraph.

The page links `app.css` (the app's tokens and type scale), has a table of
contents from the `##`/`###` headings, and a few lines of script that put
the page's own origin in place of `$DMD` in every code block (so a copied
example runs against the player that served it) and add a Copy button.
"""
import html
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

# Escape for HTML, but leave an entity the Markdown already wrote
# (`&lt;path&gt;` in a heading) alone.
_AMP = re.compile(r"&(?!#?\w+;)")


def esc(s: str) -> str:
    return _AMP.sub("&amp;", s).replace("<", "&lt;").replace(">", "&gt;")


def inline(s: str) -> str:
    out = []
    # Code spans first: nothing inside them is Markdown.
    for i, part in enumerate(re.split(r"(`[^`]*`)", s)):
        if i % 2:
            out.append(f"<code>{esc(part[1:-1])}</code>")
            continue
        t = esc(part)
        t = re.sub(r"\*\*(.+?)\*\*", r"<strong>\1</strong>", t)
        t = re.sub(r"(?<![\w*])\*(?!\s)(.+?)(?<!\s)\*(?![\w*])", r"<em>\1</em>", t)
        t = re.sub(r"\[([^\]]+)\]\(([^)\s]+)\)", r'<a href="\2">\1</a>', t)
        out.append(t)
    return "".join(out)


def slug(text: str, seen: set[str]) -> str:
    base = re.sub(r"<[^>]+>|&\w+;", "", text).lower()
    base = re.sub(r"[^a-z0-9]+", "-", base).strip("-") or "s"
    s, n = base, 2
    while s in seen:
        s, n = f"{base}-{n}", n + 1
    seen.add(s)
    return s


def cells(row: str) -> list[str]:
    """Split a table row on `|`, but not inside a code span."""
    row = row.strip().strip("|")
    out, cur, code = [], [], False
    for ch in row:
        if ch == "`":
            code = not code
        if ch == "|" and not code:
            out.append("".join(cur).strip())
            cur = []
        else:
            cur.append(ch)
    out.append("".join(cur).strip())
    return out


def convert(md: str) -> tuple[str, str, list[tuple[int, str, str]]]:
    """Body HTML, the title, and the (level, id, html) headings."""
    lines = md.split("\n")
    out: list[str] = []
    toc: list[tuple[int, str, str]] = []
    seen: set[str] = set()
    title = "API reference"
    i = 0
    para: list[str] = []

    def flush():
        if para:
            out.append(f"<p>{inline(' '.join(para))}</p>")
            para.clear()

    while i < len(lines):
        line = lines[i]
        if line.startswith("```"):
            flush()
            i += 1
            code = []
            while i < len(lines) and not lines[i].startswith("```"):
                code.append(lines[i])
                i += 1
            i += 1
            out.append(f"<pre><code>{html.escape(chr(10).join(code), quote=False)}</code></pre>")
            continue
        m = re.match(r"(#{1,3}) (.*)", line)
        if m:
            flush()
            level, text = len(m.group(1)), inline(m.group(2).strip())
            if level == 1:
                title = re.sub(r"<[^>]+>", "", text)
                out.append(f"<h1>{text}</h1>")
            else:
                hid = slug(text, seen)
                toc.append((level, hid, text))
                out.append(f'<h{level} id="{hid}">{text}</h{level}>')
            i += 1
            continue
        if line.startswith("|") and i + 1 < len(lines) and re.match(r"\|[-| :]+\|$", lines[i + 1].strip()):
            flush()
            head = cells(line)
            i += 2
            rows = []
            while i < len(lines) and lines[i].startswith("|"):
                rows.append(cells(lines[i]))
                i += 1
            t = ["<div class=\"tw\"><table><thead><tr>"]
            t += [f"<th>{inline(c)}</th>" for c in head]
            t.append("</tr></thead><tbody>")
            for r in rows:
                t.append("<tr>" + "".join(f"<td>{inline(c)}</td>" for c in r) + "</tr>")
            t.append("</tbody></table></div>")
            out.append("".join(t))
            continue
        if line.startswith("- "):
            flush()
            items = []
            while i < len(lines) and lines[i].startswith("- "):
                item = [lines[i][2:]]
                i += 1
                while i < len(lines) and lines[i].startswith("  ") and lines[i].strip():
                    item.append(lines[i].strip())
                    i += 1
                items.append(f"<li>{inline(' '.join(item))}</li>")
            out.append("<ul>" + "".join(items) + "</ul>")
            continue
        if not line.strip():
            flush()
        else:
            para.append(line.strip())
        i += 1
    flush()
    return "\n".join(out), title, toc


STYLE = """
body{height:auto}
#top a{font-size:13px;margin-left:auto}
.doc{display:grid;grid-template-columns:200px minmax(0,1fr);gap:32px;max-width:1120px;margin:0 auto;padding:60px 16px 48px}
nav.toc{position:sticky;top:60px;align-self:start;max-height:calc(100vh - 76px);overflow:auto;font-size:13px}
nav.toc a{display:block;color:var(--mut);text-decoration:none;padding:2px 0}
nav.toc a:hover{color:var(--fg)}nav.toc a.l3{padding-left:12px;font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:12px}
main{min-width:0;max-width:820px}
h1{font-size:24px;margin:0 0 16px}
h2,h3{scroll-margin-top:56px}
h2{margin:32px 0 12px;padding-top:16px;border-top:1px solid var(--line)}
h3{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;margin:24px 0 8px}
ul{padding-left:20px}li{margin:4px 0}
code{background:var(--bg3);border-radius:4px;padding:0 4px;overflow-wrap:anywhere}
pre{position:relative;background:var(--bg2);border-radius:var(--r);padding:12px 16px;overflow-x:auto;margin:8px 0 16px}
pre code{background:none;padding:0;overflow-wrap:normal;white-space:pre}
pre button{position:absolute;top:6px;right:6px;min-height:24px;padding:0 8px;font-size:12px;background:var(--bg3);opacity:.7}
pre:hover button{opacity:1}
.tw{overflow-x:auto;margin:8px 0 16px}
table{border-collapse:collapse;font-size:13px;width:100%}
th,td{text-align:left;vertical-align:top;padding:4px 8px;border-bottom:1px solid var(--line)}
th{color:var(--mut);font-weight:600}
@media (max-width:900px){.doc{grid-template-columns:minmax(0,1fr);padding:56px 12px 32px}nav.toc{position:static;max-height:none;columns:2;gap:16px}nav.toc a.l3{display:none}}
"""

# `$DMD` in every example becomes this page's origin, so what is copied runs
# against the player that served it; and a Copy button (execCommand where the
# clipboard API is missing: a player on the LAN is not a secure context).
SCRIPT = """
(()=>{const o=location.protocol.startsWith('http')?location.origin:null;
for(const c of document.querySelectorAll('pre code')){
if(o)c.textContent=c.textContent.replaceAll('DMD=http://opendmd.local','DMD='+o).replaceAll('$DMD',o);
const b=document.createElement('button');b.type='button';b.textContent='Copy';
b.onclick=async()=>{const t=c.textContent;let ok=false;
try{await navigator.clipboard.writeText(t);ok=true}catch{const a=document.createElement('textarea');a.value=t;document.body.append(a);a.select();try{ok=document.execCommand('copy')}catch{}a.remove()}
b.textContent=ok?'Copied':'Select and copy';setTimeout(()=>b.textContent='Copy',1500)};
c.parentNode.append(b)}})();
"""


def page(md: str) -> str:
    body, title, toc = convert(md)
    links = "".join(f'<a class="l{lvl}" href="#{hid}">{text}</a>' for lvl, hid, text in toc)
    return f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="dark">
<title>OpenDMD API reference</title>
<link rel="stylesheet" href="./app.css">
<style>{STYLE.strip()}</style>
</head>
<body>
<header id="top"><strong>OpenDMD</strong><span class="muted">API reference</span><a href="./">Back to the app</a></header>
<div class="doc">
<nav class="toc" aria-label="Contents">{links}</nav>
<main>
{body}
</main>
</div>
<script>{SCRIPT.strip()}</script>
</body>
</html>
"""


def main() -> int:
    src = Path(sys.argv[1]) if len(sys.argv) > 1 else REPO / "docs" / "api.md"
    dst = Path(sys.argv[2]) if len(sys.argv) > 2 else REPO / "opendmd-web" / "www" / "api.html"
    out = page(src.read_text(encoding="utf-8"))
    dst.write_text(out, encoding="utf-8")
    print(f"api-page: {src.name} -> {dst} ({len(out):,} B)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
