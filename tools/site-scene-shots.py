# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright>=1.45", "pillow==11.3.0"]
# ///
"""Photograph the home page's hero in every scene.

    uv run tools/site-scene-shots.py [SITE] [--out DIR] [--scene NAME ...] [--compare PNG]

SITE is a built site (make site; target/site by default), served from a
local port and opened as /?scene=NAME (site/theme/scene.js, SCENES.md), each
time in a browser that remembers nothing.

Writes DIR/NAME-{1280,1920,820,390}-{light,dark}.png (the hero down to the
start of the next band) and DIR/contact.png, every scene side by side;
--compare adds one more picture to that sheet (a screenshot of another
site, say), which is not kept in the repository.
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
SCENES = ("shelf", "synth", "night", "day", "chunky", "neon", "arcade")  # tools/site.py SCENES: the first is everyone's
VIEWS = (("1280", 1280, 800), ("1920", 1920, 1080), ("820", 820, 1180), ("390", 390, 844))


def serve(folder: Path):
    class Quiet(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *a):
            pass

    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Quiet, directory=str(folder)))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv


def shoot(base: str, out: Path, scenes: list[str]):
    with sync_playwright() as pw:
        browser = pw.chromium.launch()
        for scene in scenes:
            for scheme in ("light", "dark"):
                for name, w, h in VIEWS:
                    ctx = browser.new_context(viewport={"width": w, "height": h}, color_scheme=scheme)
                    page = ctx.new_page()
                    page.goto(f"{base}?scene={scene}", wait_until="load")
                    page.wait_for_timeout(2500)  # the panel's show starts, the fonts settle
                    box = page.locator(".hero").bounding_box()
                    foot = int(box["y"] + box["height"]) + (140 if w > 600 else 90)
                    page.screenshot(path=out / f"{scene}-{name}-{scheme}.png", clip={"x": 0, "y": 0, "width": w, "height": foot}, full_page=True)
                    print(out / f"{scene}-{name}-{scheme}.png", f"hero {int(box['height'])} px")
                    ctx.close()
        browser.close()


def contact(out: Path, compare: Path | None):
    """One sheet: a row per scene, its picture, the hero at 1280 (light, dark) and on a phone."""
    font = ImageFont.load_default(22)
    rows = []
    rh = 430
    for scene in SCENES:
        art = Image.open(THEME / ("den.webp" if scene == SCENES[0] else f"den-{scene}.webp")).convert("RGB")
        cells = [art] + [Image.open(out / f"{scene}-{v}.png").convert("RGB") for v in ("1280-light", "1280-dark", "390-light")]
        cells = [c.resize((round(c.width * rh / c.height), rh), Image.LANCZOS) for c in cells]
        rows.append((scene + ("   (everyone's)" if scene == SCENES[0] else f"   (?scene={scene})"), cells))
    if compare:
        c = Image.open(compare).convert("RGB")
        rows.append((f"for comparison: {compare.stem}", [c.resize((round(c.width * rh / c.height), rh), Image.LANCZOS)]))
    width = max(sum(c.width + 16 for c in cells) for _, cells in rows) + 16
    sheet = Image.new("RGB", (width, len(rows) * (rh + 52) + 16), "#17181c")
    d = ImageDraw.Draw(sheet)
    for i, (label, cells) in enumerate(rows):
        y = 16 + i * (rh + 52)
        d.text((16, y), label + ("" if len(cells) == 1 else ": the picture; the hero at 1280 light and dark; a phone"), fill="#ecebe7", font=font)
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
    ap.add_argument("--scene", action="append", choices=SCENES)
    ap.add_argument("--compare", type=Path, help="one more picture for the contact sheet")
    args = ap.parse_args()
    assert (args.site / "index.html").exists(), f"no {args.site}/index.html: make site first"
    args.out.mkdir(parents=True, exist_ok=True)
    srv = serve(args.site)
    shoot(f"http://127.0.0.1:{srv.server_address[1]}/", args.out, args.scene or list(SCENES))
    srv.shutdown()
    contact(args.out, args.compare)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
