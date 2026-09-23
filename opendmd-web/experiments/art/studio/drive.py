# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright>=1.45", "pillow"]
# ///
"""Drive the Studio prototype headless: Commons search -> logo, a local
file -> native window, a wide sheet -> pan, a synthetic paste. Writes the
panel previews (the real wasm renderer) and the GIFs to out/, prints the
stats and timings. Needs `make web` (www/pkg) and fetch_inputs.py's files.

    ART_TMP=/tmp/opendmd-art uv run opendmd-web/experiments/art/studio/drive.py
"""
import base64, functools, http.server, json, os, threading
from pathlib import Path
from playwright.sync_api import sync_playwright

HERE = Path(__file__).resolve().parent
WEB = HERE.parents[2]
SRC = Path(os.environ.get("ART_TMP", "/tmp/opendmd-art")) / "src"
OUT = HERE / "out"
OUT.mkdir(exist_ok=True)


class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *a):
        pass


srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Quiet, directory=str(WEB)))
threading.Thread(target=srv.serve_forever, daemon=True).start()
BASE = f"http://127.0.0.1:{srv.server_port}/experiments/art/studio/index.html"


def snap(page, name):
    page.wait_for_timeout(400)
    url = page.evaluate("document.getElementById('panel').toDataURL('image/png')")
    (OUT / f"{name}.png").write_bytes(base64.b64decode(url.split(",", 1)[1]))
    gif = page.evaluate("Array.from(window.studio.gif)")
    (OUT / f"{name}.gif").write_bytes(bytes(gif))
    r = page.evaluate("({stats: window.studio.stats, t: window.studio.t, name: document.getElementById('name').textContent, parsed: window.studio.parse()})")
    print(f"{name}: {json.dumps(r['stats'])}\n   timings ms {json.dumps({k: round(v, 1) for k, v in r['t'].items()})}\n   -> {r['name']} parses as {json.dumps(r['parsed'])}")


with sync_playwright() as pw:
    b = pw.chromium.launch()
    page = b.new_page(viewport={"width": 1280, "height": 1000})
    page.on("pageerror", lambda e: print("pageerror:", e))
    page.on("console", lambda m: m.type == "error" and print("console:", m.text))
    page.goto(BASE)
    page.wait_for_function("window.studioReady", timeout=30000)

    # 1. Commons search (API with origin=*, thumbnails CORS-open) -> a logo.
    page.fill("#q", "Sega Saturn logo filetype:drawing")
    page.click("#search")
    page.wait_for_selector("#results img", timeout=20000)
    titles = page.eval_on_selector_all("#results img", "els => els.map(e => e.title)")
    print("commons results:", titles[:4])
    idx = next((i for i, t in enumerate(titles) if "blue planet" in t.lower()), 0)
    page.select_option("#fit", "height")
    page.select_option("#cols", "16")
    page.locator("#results img").nth(idx).click()
    page.wait_for_function("window.studio.stats", timeout=20000)
    page.select_option("#plat", "SATURN"); page.fill("#title", "Sega Saturn logo"); page.dispatch_event("#title", "change")
    snap(page, "commons_saturn_logo")

    # A SNES-sized frame as the web usually serves it: 2x nearest-neighbour.
    from PIL import Image
    a = Image.open(SRC / "pp" / "SampleA.png").convert("RGB")
    nat = a.resize((a.width // 2, a.height // 2), Image.NEAREST).crop((100, 0, 356, 224))
    shot = SRC / "shot_2x.png"
    nat.resize((512, 448), Image.NEAREST).save(shot)

    # 2. A local file (the file picker path; nothing leaves the machine).
    page.select_option("#fit", "native"); page.select_option("#cols", "256")
    page.set_input_files("#file", str(shot))
    page.wait_for_timeout(1500)
    page.select_option("#motion", "still"); page.dispatch_event("#motion", "change")
    page.select_option("#plat", "SNES"); page.fill("#title", "Kenney Platformer"); page.dispatch_event("#title", "change")
    snap(page, "file_native_window")
    page.check("#key"); page.wait_for_timeout(800)
    page.select_option("#motion", "still")
    snap(page, "file_native_window_keyed")

    # 3. A wide sheet -> pan (auto-selected for wide, short sources).
    page.uncheck("#key")
    page.set_input_files("#file", str(SRC / "pp" / "Tilemap" / "tilemap-backgrounds_packed.png"))
    page.wait_for_timeout(1500)
    page.select_option("#motion", "pan")
    page.select_option("#plat", "GB"); page.fill("#title", "Kenney Backgrounds"); page.dispatch_event("#title", "change")
    snap(page, "file_wide_pan")

    # 4. Paste (synthetic ClipboardEvent: proves the handler, not the OS clipboard).
    data = base64.b64encode((SRC / "snes.png").read_bytes()).decode()
    page.select_option("#fit", "height"); page.select_option("#cols", "16")
    page.evaluate("""(b64) => { const bytes = Uint8Array.from(atob(b64), c => c.charCodeAt(0));
      const dt = new DataTransfer(); dt.items.add(new File([bytes], 'x.png', {type: 'image/png'}));
      window.dispatchEvent(new ClipboardEvent('paste', {clipboardData: dt})); }""", data)
    page.wait_for_timeout(1500)
    page.select_option("#plat", "SNES"); page.fill("#title", "Super Nintendo"); page.dispatch_event("#title", "change")
    snap(page, "paste_snes_logo")

    # 5. A CORS-blocked host: the page's message.
    page.fill("#url", "https://img.itch.zone/aW1nLzEyMzQ1Ni5wbmc=/original/abc.png")
    page.click("#go"); page.wait_for_timeout(3000)
    print("blocked host ->", page.text_content("#info"))
    page.screenshot(path=str(OUT / "studio_page.png"), full_page=True)
    b.close()
srv.shutdown()
