# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright>=1.45", "pillow"]
# ///
"""The README's, the guide's and the site's screenshots of the web app,
taken in headless Chromium against tools/mock-device.py over the demo card
(and tools/mock-replay.py, a fake RePlayOS, for Follow).

    uv run tools/screenshots.py [--card target/demo-card] [--out docs/images]

(`make screenshots` builds the app and the demo card first.) Writes, at
1280x800: now.png, formulas.png and playlists.png; create-art.png (1280
wide, the whole page); at 2x, the part that matters in a 1280x800 window
(the site's bands): library.png (Select, the playlist menu open),
now-follow.png (Now while following RePlayOS, the title card up),
settings-look.png (Settings › Panel) and settings-schedule.png (Settings ›
Schedule: a Christmas rule, an evening rule, night mode from sunset to
sunrise); and at 2x, phone-now.png (390x700) and phone-library.png
(390x844).

The card must be tools/demo-card.py's: Formulas recorded as GIFs and the
CC0 Kenney sprites, nothing from the DMDoS pack and no game art (these
images are published). No shot may show a card path, "odm", "bucket" or
"cache": the Library's details show the GIF's file name, so no shot has
them open. Writes go to a throwaway overlay; the card is left as it was.
Needs `make web` and, once, `uv run --with playwright playwright install
chromium`.
"""

import argparse
import json
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

from PIL import Image
from playwright.sync_api import Page, sync_playwright

ROOT = Path(__file__).resolve().parents[1]
KENNEY_WALK = "/gifs/Kenney/OTHER_Pixel_Platformer_Walk_01_kenney.gif"
KENNEY_PAN = "/gifs/Kenney/OTHER_Pixel_Platformer_Pan_01_kenney.gif"
KENNEY_SCROLL = "/gifs/Kenney/OTHER_Pixel_Platformer_Scroll_01_kenney.gif"
PHONE = {"viewport": {"width": 390, "height": 844}, "device_scale_factor": 2, "is_mobile": True, "has_touch": True}
LAPTOP = {"viewport": {"width": 1280, "height": 800}}


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def wait_port(proc: subprocess.Popen, port: int, what: str) -> None:
    for _ in range(150):
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                return
        except OSError:
            time.sleep(0.1)
    proc.kill()
    sys.exit(f"{what} did not come up")


def start_mock(card: Path, overlay: Path) -> tuple[subprocess.Popen, str]:
    port = free_port()
    proc = subprocess.Popen(["uv", "run", str(ROOT / "tools/mock-device.py"), "--port", str(port), "--quiet",
                             "--root", str(card), "--overlay", str(overlay), "--dwell", "600"],
                            stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT)
    wait_port(proc, port, "mock-device.py")
    return proc, f"http://127.0.0.1:{port}"


def start_replay(token: str) -> tuple[subprocess.Popen, str, int]:
    """A fake RePlayOS, parked in its menu until told otherwise."""
    port = free_port()
    proc = subprocess.Popen(["uv", "run", str(ROOT / "tools/mock-replay.py"), "--port", str(port), "--token", token,
                             "--speed", "0.01", "--quiet"],
                            stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT)
    wait_port(proc, port, "mock-replay.py")
    return proc, f"http://127.0.0.1:{port}", port


def call(base: str, method: str, path: str, body=None) -> None:
    data = b"" if body is None else json.dumps(body).encode()
    req = urllib.request.Request(base + path, data=data, method=method, headers={"Content-Type": "application/json"})
    urllib.request.urlopen(req, timeout=10).read()


def page_for(browser, size: dict) -> Page:
    # bypass_csp: Playwright's wait_for_function evaluates strings, which the
    # app's Content-Security-Policy refuses (as in tools/web-smoke.py). The
    # banner asking for an API key is put off ("Not now"), as a reader would.
    ctx = browser.new_context(**size, bypass_csp=True)
    ctx.add_init_script("try { sessionStorage.setItem('odmd.keynag', 'later'); } catch {}")
    return ctx.new_page()


def status(base: str) -> dict:
    with urllib.request.urlopen(base + "/api/status", timeout=10) as r:
        return json.load(r)


def reboot(base: str) -> None:
    """A restart (Now's count and Recently played start again), then the first GIF."""
    call(base, "POST", "/api/reboot")
    for _ in range(100):
        time.sleep(0.2)
        try:
            if status(base)["now_playing"]["path"]:
                time.sleep(0.8)
                return
        except (OSError, KeyError):
            pass
    sys.exit("the mock did not come back from its reboot")


def skip_to(base: str, path: str, at_least: int) -> None:
    """Skip through the playlist (filling Recently played) until `path` is on."""
    for n in range(40):
        if n >= at_least and status(base)["now_playing"]["path"] == path:
            time.sleep(1.0)
            return
        call(base, "POST", "/api/skip")
        time.sleep(0.6)
    sys.exit(f"never reached {path}")


def wait_lit(page: Page, canvas: str, timeout_s: float = 20) -> None:
    """Until the preview shows something (and a moment more for the glow)."""
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        n = page.evaluate("""(sel) => {
            const c = document.querySelector(sel);
            if (!c || !c.width) return 0;
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


def select(page: Page, paths: list[str]) -> None:
    """The Library's Select, with `paths` ticked."""
    page.evaluate("async (ps) => { const { app } = await import('./js/state.js'); ps.forEach((p) => app.selection.add(p)); }", paths)
    page.click("#lib-select")
    page.wait_for_function(f"document.querySelector('#lib-selbar b')?.textContent === '{len(paths)}'", timeout=10000)
    page.wait_for_timeout(400)


def now_ready(page: Page, path: str) -> None:
    page.wait_for_function(f"document.querySelector('#now-path')?.textContent === {json.dumps(path)}", timeout=20000)
    wait_lit(page, "#now-panel")
    page.wait_for_selector("#now-recent > div", timeout=10000)
    page.wait_for_timeout(1500)


def shot(page: Page, out: Path, name: str, full: bool = False) -> None:
    page.evaluate("document.activeElement?.blur?.(); document.querySelectorAll('#toasts .toast').forEach(t => t.remove())")
    page.mouse.move(0, 0)
    page.wait_for_timeout(200)
    page.screenshot(path=str(out / name), full_page=full)
    print(f"  {out / name}")


def clip(page: Page, out: Path, name: str, parts: str, pad: int = 12) -> None:
    """A shot of just the elements the JS expression `parts` gives (their
    bounding box, `pad` px of the page around it), at the page's scale."""
    page.evaluate("document.activeElement?.blur?.(); document.querySelectorAll('#toasts .toast').forEach(t => t.remove())")
    page.mouse.move(0, 0)
    page.wait_for_timeout(200)
    box = page.evaluate(f"""(pad) => {{
        const rs = ({parts}).filter(Boolean).map(e => e.getBoundingClientRect());
        const x = Math.min(...rs.map(r => r.left)) - pad, y = Math.min(...rs.map(r => r.top)) - pad;
        return {{x: x + scrollX, y: y + scrollY, width: Math.max(...rs.map(r => r.right)) + pad - x, height: Math.max(...rs.map(r => r.bottom)) + pad - y}};
    }}""", pad)
    page.screenshot(path=str(out / name), full_page=True, clip=box)
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
    # A Christmas list beside the card's own (the schedule offers its rule).
    (overlay / "filelist.xmas").write_text("\n".join((KENNEY_WALK, KENNEY_PAN, KENNEY_SCROLL)) + "\n")
    proc, base = start_mock(a.card, overlay)
    replay = None
    try:
        # The name the site and README show; the mock's own is opendmd-mock.
        # Now's "N of M" counts the GIFs started since boot: a reboot after
        # the settings, then a skip or two through All files (in order).
        call(base, "PUT", "/api/config", {"hostname": "opendmd", "set": "", "shuffle": False})
        reboot(base)
        with sync_playwright() as pw:
            browser = pw.chromium.launch()
            # Now on a phone, then on a laptop: the Kenney sprites, the ones
            # before them in Recently played, what comes next.
            # (A shorter phone for Now: its page ends above the fold.)
            phone = page_for(browser, {**PHONE, "viewport": {"width": 390, "height": 700}})
            skip_to(base, KENNEY_PAN, 1)
            phone.goto(base + "/#now")
            now_ready(phone, KENNEY_PAN)
            shot(phone, a.out, "phone-now.png")
            phone.context.close()
            page = page_for(browser, LAPTOP)
            skip_to(base, KENNEY_SCROLL, 1)
            page.goto(base + "/#now")
            now_ready(page, KENNEY_SCROLL)
            shot(page, a.out, "now.png")
            # Sprites playing from here: it heads the playlist menu.
            call(base, "PUT", "/api/config", {"set": "Sprites"})
            time.sleep(1.5)
            page.reload()
            # The site's bands: the parts that matter, at twice the scale.
            # Library: the Kenney folder, two GIFs ticked, the playlist menu open
            # (a shorter window: the list fills it).
            big = page_for(browser, {"viewport": {"width": 1280, "height": 560}, "device_scale_factor": 2})
            big.goto(base + "/#library")
            folder(big, "Kenney")
            select(big, [KENNEY_WALK, KENNEY_SCROLL])
            big.click("#lib-to-set")
            big.wait_for_function("[...document.querySelectorAll('#pl-target [data-path] small')].every(s => s.textContent)", timeout=10000)
            big.wait_for_timeout(300)
            clip(big, a.out, "library.png", "[document.querySelector('#lib-search')?.closest('.card') ?? document.querySelector('#lib-select').closest('.card'), "
                                            "document.querySelector('#lib-list'), document.querySelector('#lib-selbar'), document.querySelector('#pl-target')]")
            big.context.close()
            big = page_for(browser, {**LAPTOP, "device_scale_factor": 2})
            # Formulas: the gallery, the first example (Plasma) in Details.
            go(page, "formulas")
            page.wait_for_selector("#psi-ex .psi-tile", timeout=20000)
            wait_lit(page, "#psi-panel")
            page.wait_for_timeout(800)
            shot(page, a.out, "formulas.png")
            # Playlists: Sprites, the one playing, open in the editor.
            go(page, "sets")
            page.click("#sets-list [data-path='/sets/Sprites.txt'] button.pl")
            page.wait_for_selector("#set-entries .vrow[data-path]", timeout=20000)
            page.wait_for_timeout(1200)
            shot(page, a.out, "playlists.png")
            # Settings › Panel: amber with a soft glow.
            call(base, "PUT", "/api/config", {"look": "amber", "glow": "short"})
            big.goto(base + "/#settings/look")
            big.reload()
            big.wait_for_selector("#st-tab-look #st-glow", timeout=20000)
            big.wait_for_selector("#st-power-card *", timeout=10000)
            big.wait_for_timeout(1200)
            clip(big, a.out, "settings-look.png", "[document.querySelector('#st-tab-look > .card')]")
            call(base, "PUT", "/api/config", {"look": "colour", "glow": "off"})
            # Settings › Schedule: Christmas from 1 to 26 December, weekday
            # evenings, and night mode dimming from sunset to sunrise.
            call(base, "PUT", "/api/config", {"tz": "GMT0IST,M3.5.0/1,M10.5.0", "tz_name": "Europe/Dublin", "lat": 53.35, "lon": -6.26})
            call(base, "PUT", "/api/schedule", {
                "rules": [{"days": [], "from": "00:00", "to": "00:00", "set": "filelist.xmas", "dates": ["12-01", "12-26"]},
                          {"days": [1, 2, 3, 4, 5], "from": "18:00", "to": "23:00", "set": "Demoscene"}],
                "night": {"from": "23:30", "to": "07:00", "sun": True, "offset": 30, "mode": "dim", "brightness": 8}})
            big.goto(base + "/#settings/time")
            big.reload()
            big.wait_for_selector("#sc-rules [data-rule='1']", timeout=20000)
            big.wait_for_timeout(1000)
            clip(big, a.out, "settings-schedule.png", "[document.querySelector('#sc')]")
            big.context.close()
            call(base, "PUT", "/api/schedule", {"rules": [], "night": None})
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
            page.context.close()

            # The Library on a phone, GIFs ticked.
            phone = page_for(browser, PHONE)
            phone.goto(base + "/#library")
            phone.wait_for_selector("#lib-list", timeout=20000)
            folder(phone, "Kenney")
            select(phone, [KENNEY_WALK, KENNEY_PAN])
            shot(phone, a.out, "phone-library.png")
            phone.context.close()

            # Following RePlayOS: a game starts on the box, and Now shows its
            # title card with the line under the title.
            call(base, "PUT", "/api/config", {"set": ""})
            reboot(base)
            replay, rbase, rport = start_replay("4321")
            call(rbase, "POST", "/mock/goto?step=0")
            call(base, "PUT", "/api/config", {"follow": True, "follow_host": f"localhost:{rport}", "follow_code": "4321"})
            page = page_for(browser, {**LAPTOP, "device_scale_factor": 2})
            page.goto(base + "/#now")
            page.wait_for_function("document.querySelector('#now-follow-line')?.textContent.includes('attract')", timeout=30000)
            call(rbase, "POST", "/mock/goto?step=1")
            page.wait_for_function("document.querySelector('#now-follow-line')?.textContent.includes('title card')", timeout=30000)
            wait_lit(page, "#now-panel", 5)
            clip(page, a.out, "now-follow.png", "[document.querySelector('#now-panel').closest('.card')]")
            page.context.close()
            browser.close()
    finally:
        proc.terminate()
        if replay:
            replay.terminate()
        shutil.rmtree(overlay, ignore_errors=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
