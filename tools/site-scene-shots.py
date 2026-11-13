# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright>=1.45", "pillow==11.3.0"]
# ///
"""Photograph the home page's hero with each candidate scene swapped in.

    uv run tools/site-scene-shots.py [SITE] [--out DIR] [--style NAME ...] [--compare PNG]

SITE is a built site (make site; target/site by default). Nothing in it is
changed: the page is served from a local port, the requests for den.webp
and den-phone.webp are answered with site/theme/den-NAME*.webp, and the few
lines of CSS a style needs (HERO_CSS: the wall colour behind the picture,
the shade behind the words, the ink on a light wall) are added to the page.
Those lines are what site.css would take on if the owner picks the style.

Writes DIR/NAME-{1280,1920,390}-{light,dark}.png (the hero down to the
start of the next band) and DIR/contact.png, every style side by side with
the scene the site has now; --compare adds one more picture to that sheet
(a screenshot of another site, say), which is not kept in the repository.
"""
import argparse
import functools
import http.server
import threading
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont
from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parent.parent
THEME = ROOT / "site/theme"


def dark(wall: str, shade: str, bar: str, line: str, extra: str = "") -> str:
    r, g, b = (int(shade[i:i + 2], 16) for i in (1, 3, 5))
    return f""":root{{--den:{wall}}}
.home .top{{background:{bar};border-bottom-color:{line}}}.home .topnav a:hover{{background:{line}}}
.hero::before{{background:radial-gradient(ellipse 35% 44% at 50% 54%,rgba({r},{g},{b},.9),rgba({r},{g},{b},.66) 55%,rgba({r},{g},{b},0) 100%)}}
@media (max-width:1180px){{.hero::before{{background:radial-gradient(ellipse 58% 46% at 50% 56%,rgba({r},{g},{b},.93),rgba({r},{g},{b},.8) 60%,rgba({r},{g},{b},0) 100%)}}}}
.hero .cta.alt{{background:rgba({r},{g},{b},.6)}}
.hero-panel::before,.hero-panel::after{{background:{line};box-shadow:3px 0 0 {bar}}}
{extra}"""


def light(wall: str, bar: str, line: str, extra: str = "") -> str:
    r, g, b = (int(wall[i:i + 2], 16) for i in (1, 3, 5))
    return f""":root{{--den:{wall};--den-fg:#1c1d20;--den-mut:#5a5648}}
.home .top{{background:{bar};border-bottom-color:{line}}}.home .topnav a:hover{{background:{line}}}
.home .mnav summary{{border-color:{line}}}.home .mnav[open] summary{{background:{line}}}
.hero::before{{background:radial-gradient(ellipse 35% 44% at 50% 54%,rgba({r},{g},{b},.94),rgba({r},{g},{b},.8) 55%,rgba({r},{g},{b},0) 100%)}}
@media (max-width:1180px){{.hero::before{{background:radial-gradient(ellipse 58% 46% at 50% 56%,rgba({r},{g},{b},.96),rgba({r},{g},{b},.88) 60%,rgba({r},{g},{b},0) 100%)}}}}
.hero .lead{{color:#3a3a3d}}
.hero .cta.alt{{background:rgba(255,255,255,.7);box-shadow:inset 0 0 0 1.5px #8a8576}}.hero .cta.alt:hover{{box-shadow:inset 0 0 0 1.5px #1c1d20}}
.hero :focus-visible,.hero-panel .bezel[data-live]:focus-visible{{outline-color:#1c1d20}}
.proof{{background-image:radial-gradient(circle,rgba(28,29,32,.4) 1.2px,transparent 1.8px)}}
.hero-panel::before,.hero-panel::after{{background:#8a8576;box-shadow:3px 0 0 rgba(0,0,0,.18)}}
.hero-panel .bezel{{box-shadow:0 0 0 1px #2e3454,inset 0 0 0 1px #000,0 18px 40px rgba(60,40,10,.3)}}
{extra}"""


# What each style asks of site.css. None: the page as it is.
HERO_CSS = {
    "current": None,
    "day": light("#fdf8e7", "#eef3dc", "#cfe3c8"),
    "chunky": dark("#1d5c6e", "#103540", "#0e1b26", "#184e5f"),
    "shelf": light("#f6ecd2", "#eddfbd", "#d8c59a"),
    "neon": dark("#150b2e", "#0c061c", "#05020e", "#3a1d63"),
    "arcade": dark("#231942", "#150f2b", "#0d0a1c", "#2d2157"),
    "synth": dark("#0d0221", "#0d0221", "#070114", "#360f66",
                  ".hero-panel .bezel{box-shadow:0 0 0 1px #6a1687,inset 0 0 0 1px #000,0 24px 60px rgba(0,0,0,.6),0 0 90px rgba(255,46,151,.22),0 0 40px rgba(45,226,230,.18)}"
                  "@media (min-width:1181px){.hero::before{background:radial-gradient(ellipse 35% 34% at 50% 45%,rgba(13,2,33,.9),rgba(13,2,33,.66) 55%,rgba(13,2,33,0) 100%)}}"
                  ".hero{padding-bottom:clamp(250px,21vw,340px)}@media (max-width:600px){.hero{padding-bottom:calc(74vw + 8px)}}"),
}
VIEWS = (("1280", 1280, 800), ("1920", 1920, 1080), ("390", 390, 844))


def serve(folder: Path):
    class Quiet(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *a):
            pass

    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Quiet, directory=str(folder)))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv


def swap(body: bytes, route):
    route.fulfill(body=body, content_type="image/webp")


def shoot(base: str, out: Path, styles: list[str]):
    with sync_playwright() as pw:
        browser = pw.chromium.launch()
        for style in styles:
            for scheme in ("light", "dark"):
                for name, w, h in VIEWS:
                    page = browser.new_page(viewport={"width": w, "height": h}, color_scheme=scheme)
                    if style != "current":
                        for suffix in ("", "-phone"):
                            body = (THEME / f"den-{style}{suffix}.webp").read_bytes()
                            page.route(f"**/den{suffix}.webp", functools.partial(swap, body))
                    page.goto(base, wait_until="load")
                    if HERO_CSS[style]:
                        page.add_style_tag(content=HERO_CSS[style])
                    page.wait_for_timeout(2500)  # the panel's show starts, the fonts settle
                    box = page.locator(".hero").bounding_box()
                    foot = int(box["y"] + box["height"]) + (140 if w > 600 else 90)
                    page.screenshot(path=out / f"{style}-{name}-{scheme}.png", clip={"x": 0, "y": 0, "width": w, "height": foot}, full_page=True)
                    print(out / f"{style}-{name}-{scheme}.png", f"hero {int(box['height'])} px")
                    page.close()
        browser.close()


def contact(out: Path, styles: list[str], compare: Path | None):
    """One sheet: a row per style, its picture, the hero at 1280 (light, dark) and on a phone."""
    font = ImageFont.load_default(22)
    rows = []
    rh = 430
    for style in styles:
        art = Image.open(THEME / ("den.webp" if style == "current" else f"den-{style}.webp")).convert("RGB")
        cells = [art] + [Image.open(out / f"{style}-{v}.png").convert("RGB") for v in ("1280-light", "1280-dark", "390-light")]
        cells = [c.resize((round(c.width * rh / c.height), rh), Image.LANCZOS) for c in cells]
        rows.append((style, cells))
    if compare:
        c = Image.open(compare).convert("RGB")
        rows.append((f"for comparison: {compare.stem}", [c.resize((round(c.width * rh / c.height), rh), Image.LANCZOS)]))
    width = max(sum(c.width + 16 for c in cells) for _, cells in rows) + 16
    sheet = Image.new("RGB", (width, len(rows) * (rh + 52) + 16), "#17181c")
    d = ImageDraw.Draw(sheet)
    for i, (label, cells) in enumerate(rows):
        y = 16 + i * (rh + 52)
        d.text((16, y), label + ("" if len(cells) == 1 else "   (the picture; the hero at 1280 light and dark; a phone)"), fill="#ecebe7", font=font)
        x = 16
        for c in cells:
            sheet.paste(c, (x, y + 34))
            x += c.width + 16
    sheet.save(out / "contact.png")
    print(out / "contact.png", sheet.size)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("site", nargs="?", type=Path, default=ROOT / "target/site")
    ap.add_argument("--out", type=Path, default=ROOT / "target/hero-art")
    ap.add_argument("--style", action="append", choices=sorted(HERO_CSS))
    ap.add_argument("--compare", type=Path, help="one more picture for the contact sheet")
    ap.add_argument("--sheet-only", action="store_true", help="rebuild contact.png from the screenshots already in --out")
    args = ap.parse_args()
    assert (args.site / "index.html").exists(), f"no {args.site}/index.html: make site first"
    args.out.mkdir(parents=True, exist_ok=True)
    if not args.sheet_only:
        srv = serve(args.site)
        shoot(f"http://127.0.0.1:{srv.server_address[1]}/", args.out, args.style or list(HERO_CSS))
        srv.shutdown()
    contact(args.out, list(HERO_CSS), args.compare)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
