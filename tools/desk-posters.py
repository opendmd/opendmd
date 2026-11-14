# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright>=1.45", "pillow"]
# ///
"""The 3D desk scene's stills (opendmd-web/share/desk/hd/poster-day.webp, poster-night.webp):
photographs of the scene itself, the panel dark, shown as the scene's background until its
first frame. Retake them when the look changes (after make site):

    uv run tools/desk-posters.py [target/site]

Day is the light theme's lights (100), night the dark theme's (6); the best quality level; 720
pixels wide, WebP quality 52 (tools/web-size.py's desk lines: <= 20 KB each).
"""
import functools
import http.server
import io
import sys
import threading
from pathlib import Path

from PIL import Image
from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "opendmd-web/share/desk/hd"


def main() -> int:
    site = Path(sys.argv[1] if len(sys.argv) > 1 else ROOT / "target/site")

    class Quiet(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *a):
            pass
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Quiet, directory=str(site)))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_port}"
    with sync_playwright() as p:
        browser = p.chromium.launch()
        for name, light, scheme in (("day", 100, "light"), ("night", 6, "dark")):
            ctx = browser.new_context(viewport={"width": 1440, "height": 900}, color_scheme=scheme)
            page = ctx.new_page()
            page.goto(f"{base}/demo/?scene=hd&q=0&light={light}")
            page.wait_for_function("window.desk && desk.dev && desk.S.W && desk.S.W.up && desk.scene && desk.scene.kind == 'hd'", timeout=120000)
            # the panel dark, the charger still in: the player stopped, its LEDs cleared
            page.evaluate("desk.dev.run(false); desk.dev.frame().getContext('2d').clearRect(0, 0, 128, 32); desk.dev.frame().getContext('2d').fillRect(0, 0, 128, 32)")
            page.wait_for_timeout(4000)
            png = page.locator("#scene").screenshot()
            im = Image.open(io.BytesIO(png)).convert("RGB")
            im = im.resize((720, round(720 * im.height / im.width)), Image.LANCZOS)
            f = OUT / f"poster-{name}.webp"
            im.save(f, "WEBP", quality=52, method=6)
            print(f"{f}: {im.width}x{im.height}, {f.stat().st_size} bytes")
            ctx.close()
        browser.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
