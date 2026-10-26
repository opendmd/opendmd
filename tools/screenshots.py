# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright>=1.45", "pillow"]
# ///
"""The README's and the site's screenshots of the web app, taken in
headless Chromium against tools/mock-device.py over the demo card.

    uv run tools/screenshots.py [--card target/demo-card] [--out docs/images]

(`make screenshots` builds the app and the demo card first.) Writes
library.png, effects.png, formulas.png, now.png, playlists.png (1280x800),
create-art.png (1280 wide, the whole page) and phone-library.png (390x844
at 2x). The card must be tools/demo-card.py's: the panel's own effects and
live items and the CC0 Kenney sprites, nothing from the DMDoS pack and no
game art — these images are published. Writes go to a throwaway overlay;
the card is left as it was. Needs `make web` and, once,
`uv run --with playwright playwright install chromium`.
"""

import argparse
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
import json
from pathlib import Path

from PIL import Image
from playwright.sync_api import Page, sync_playwright

ROOT = Path(__file__).resolve().parents[1]
KENNEY_WALK = "/gifs/Kenney/OTHER_Pixel_Platformer_Walk_01_kenney.gif"
KENNEY_SCROLL = "/gifs/Kenney/OTHER_Pixel_Platformer_Scroll_01_kenney.gif"


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def start_mock(card: Path, overlay: Path) -> tuple[subprocess.Popen, str]:
    port = free_port()
    proc = subprocess.Popen(["uv", "run", str(ROOT / "tools/mock-device.py"), "--port", str(port), "--quiet",
                             "--root", str(card), "--overlay", str(overlay), "--dwell", "600"],
                            stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT)
    for _ in range(150):
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                return proc, f"http://127.0.0.1:{port}"
        except OSError:
            time.sleep(0.1)
    proc.kill()
    sys.exit("mock-device.py did not come up")


def call(base: str, method: str, path: str, body: dict) -> None:
    req = urllib.request.Request(base + path, data=json.dumps(body).encode(), method=method,
                                 headers={"Content-Type": "application/json"})
    urllib.request.urlopen(req, timeout=10).read()


def wait_lit(page: Page, canvas: str, timeout_s: float = 20) -> None:
    """Until the preview shows something (and a moment more for the glow)."""
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        n = page.evaluate("""(sel) => {
            const c = document.querySelector(sel);
            const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
            let n = 0;
            for (let i = 0; i < d.length; i += 4) if (d[i] > 60 || d[i+1] > 60 || d[i+2] > 60) n++;
            return n;
        }""", canvas)
        if n > 200:
            page.wait_for_timeout(600)
            return
        page.wait_for_timeout(250)
    sys.exit(f"{canvas}: the preview never lit")


def go(page: Page, view: str) -> None:
    page.evaluate(f"location.hash = '#{view}'")
    page.wait_for_selector(f"#view-{view.split('/')[0]}.on")
    page.wait_for_timeout(500)


def folder(page: Page, cat: str) -> None:
    """The Library with the folder `cat` alone open, at the top (`#library/<cat>`)."""
    page.evaluate(f"location.hash = '#library/' + encodeURIComponent({json.dumps(cat)})")
    page.wait_for_function(f"(t => t && t.open.has({json.dumps(cat)}) && t.rows.some(r => r.path))(document.querySelector('#lib-list')?._tree)", timeout=20000)


def row(page: Page, path: str) -> None:
    sel = f"#lib-list .vrow:not([hidden])[data-path='{path}']"
    page.wait_for_selector(sel, timeout=20000)
    page.click(sel)


def shot(page: Page, out: Path, name: str, full: bool = False) -> None:
    page.evaluate("document.activeElement?.blur?.(); document.querySelectorAll('#toasts .toast').forEach(t => t.remove())")
    page.mouse.move(0, 0)
    page.wait_for_timeout(200)
    page.screenshot(path=str(out / name), full_page=full)
    print(f"  {out / name}")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--card", type=Path, default=ROOT / "target/demo-card")
    ap.add_argument("--out", type=Path, default=ROOT / "docs/images")
    a = ap.parse_args()
    if not (a.card / "index.json").exists():
        sys.exit(f"{a.card}: not a demo card (uv run tools/demo-card.py {a.card})")
    a.out.mkdir(parents=True, exist_ok=True)
    overlay = Path(tempfile.mkdtemp(prefix="opendmd-shots-"))
    proc, base = start_mock(a.card, overlay)
    try:
        # The name the site and README show; the mock's own is opendmd-mock.
        call(base, "PUT", "/api/config", {"hostname": "opendmd"})
        with sync_playwright() as pw:
            browser = pw.chromium.launch()
            page = browser.new_page(viewport={"width": 1280, "height": 800})
            # Now: a Kenney sprite on the panel, a few things before it in Recently played.
            for p in ("@fx:tunnel?palette=fire&texture=rings&twist=2", "@fx:wire?shape=tesseract&color=ff66cc", KENNEY_WALK):
                call(base, "POST", "/api/play", {"path": p})
                time.sleep(0.6)
            page.goto(base + "/#now")
            page.wait_for_function(f"document.querySelector('#now-path')?.textContent === {json.dumps(KENNEY_WALK)}", timeout=20000)
            wait_lit(page, "#now-panel")
            page.wait_for_selector("#now-recent > div", timeout=10000)
            page.wait_for_timeout(1500)
            shot(page, a.out, "now.png")
            # Library: Demoscene, a tunnel in Details.
            go(page, "library")
            folder(page, "Demoscene")
            row(page, "/gifs/Demoscene/FX_Tunnel_Fire_Rings_01.gif")
            wait_lit(page, "#lib-panel")
            shot(page, a.out, "library.png")
            # Effects: the tunnel and its options.
            folder(page, "@fx")
            page.wait_for_selector("#lib-list .vrow:not([hidden])[data-path^='@fx:tunnel']", timeout=20000)
            page.click("#lib-list .vrow:not([hidden])[data-path^='@fx:tunnel']")
            page.select_option("#fx-tunnel select[data-key=palette]", "fire")
            wait_lit(page, "#lib-panel")
            page.wait_for_timeout(800)
            shot(page, a.out, "effects.png")
            # Formulas: the gallery, the first example (Plasma) in Details.
            go(page, "formulas")
            page.wait_for_selector("#psi-ex .psi-tile", timeout=20000)
            wait_lit(page, "#psi-panel")
            page.wait_for_timeout(800)
            shot(page, a.out, "formulas.png")
            # Playlists: one open in the editor.
            go(page, "sets")
            page.click("#sets-list [data-path='/sets/Demoscene.txt'] button.pl")
            page.wait_for_selector("#set-entries .vrow[data-path]", timeout=20000)
            page.wait_for_timeout(500)
            shot(page, a.out, "playlists.png")
            # Create Art: a still of the Kenney scenery (a frame of the
            # scroll with the most colours), titled and filed under Kenney; the whole page.
            go(page, "create")
            page.wait_for_selector("#st-src", timeout=30000)
            page.click("#st-src [data-v=own]")
            with tempfile.TemporaryDirectory() as tmp:
                f = Path(tmp) / "Pixel_Platformer.png"
                im = Image.open(a.card / KENNEY_SCROLL.lstrip("/"))
                frames = []
                for i in range(im.n_frames):  # the busiest frame: the most colours
                    im.seek(i)
                    frames.append((len(im.convert("RGB").getcolors(4096) or ()), i))
                im.seek(max(frames)[1])
                im.convert("RGB").save(f)
                page.set_input_files("#st-file", str(f))
                page.wait_for_function("document.querySelector('#studio').dataset.frames", timeout=30000)
            page.fill("#st-title", "Pixel Platformer")
            page.select_option("#st-cat", "Kenney")
            page.wait_for_timeout(1500)
            page.evaluate("scrollTo(0, 0)")
            # The rail is fixed: a viewport as tall as the page, not a full-page shot.
            page.set_viewport_size({"width": 1280, "height": page.evaluate("document.documentElement.scrollHeight")})
            page.wait_for_timeout(500)
            shot(page, a.out, "create-art.png")
            browser.close()
            # The Library on a phone, a GIF's details open.
            browser = pw.chromium.launch()
            phone = browser.new_page(viewport={"width": 390, "height": 844}, device_scale_factor=2, is_mobile=True, has_touch=True)
            phone.goto(base + "/#library")
            phone.wait_for_selector("#lib-list", timeout=20000)
            folder(phone, "Demoscene")
            row(phone, "/gifs/Demoscene/FX_Tunnel_Chrono_01.gif")
            phone.wait_for_selector("#lib-side.open", timeout=10000)
            wait_lit(phone, "#lib-panel")
            shot(phone, a.out, "phone-library.png")
            browser.close()
    finally:
        proc.terminate()
        shutil.rmtree(overlay, ignore_errors=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
