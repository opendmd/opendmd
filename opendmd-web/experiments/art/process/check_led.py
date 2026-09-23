# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright>=1.45", "pillow", "numpy"]
# ///
"""Check led.py against the real renderer: the wasm `Preview` from
opendmd-web/www/pkg (needs `make web` once), on a 1024-px canvas (scale 8),
playing a one-frame test GIF. Prints the per-channel difference.

    uv run opendmd-web/experiments/art/process/check_led.py
"""
import base64, functools, http.server, io, sys, threading
from pathlib import Path
import numpy as np
from PIL import Image
from playwright.sync_api import sync_playwright

HERE = Path(__file__).resolve().parent
WEB = HERE.parents[2]  # opendmd-web/
sys.path.insert(0, str(HERE))
from led import Led  # noqa: E402

# Test pattern: hue bars, a grey ramp, lone dots, a white block.
y, x = np.mgrid[0:32, 0:128]
img = np.zeros((32, 128, 3), np.uint8)
bars = [(255, 0, 0), (0, 255, 0), (0, 0, 255), (255, 255, 0), (0, 255, 255), (255, 0, 255), (255, 255, 255), (255, 128, 0)]
for n, c in enumerate(bars):
    img[0:10, n * 16 : n * 16 + 16] = c
img[12:20] = (np.repeat((x[0] // 8) * 17, 1)[None, :, None]).astype(np.uint8)
img[24, ::6] = (255, 255, 255)
img[26:32, 100:120] = (200, 60, 20)
pal = Image.fromarray(img).quantize(colors=256, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.NONE)
exact = np.asarray(pal.convert("RGB"))
buf = io.BytesIO()
pal.save(buf, "GIF")
gif_b64 = base64.b64encode(buf.getvalue()).decode()

PAGE = """<!doctype html><canvas id=c width=1024 height=256></canvas><script type=module>
import init, { Preview } from '/www/pkg/opendmd_web.js';
await init();
const c = document.getElementById('c');
const p = new Preview(c, 2.2, 1.0, 0.22, false);
const bytes = Uint8Array.from(atob(window.GIF), (ch) => ch.charCodeAt(0));
p.load('/gifs/Test/T.gif', bytes);
let r = 0; for (let i = 0; i < 50 && r !== 1; i++) { r = p.frame(); await new Promise((f) => setTimeout(f, 20)); }
window.RESULT = { w: c.width, h: c.height, r, png: c.toDataURL('image/png') };
</script>"""


class H(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_GET(self):
        if self.path == "/check.html":
            b = PAGE.encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/html")
            self.end_headers()
            self.wfile.write(b)
        else:
            super().do_GET()


srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(H, directory=str(WEB)))
threading.Thread(target=srv.serve_forever, daemon=True).start()
with sync_playwright() as pw:
    b = pw.chromium.launch()
    page = b.new_page()
    page.on("console", lambda m: print("console:", m.text))
    page.on("pageerror", lambda e: print("pageerror:", e))
    page.add_init_script(f"window.GIF = '{gif_b64}';")
    page.goto(f"http://127.0.0.1:{srv.server_port}/check.html")
    page.wait_for_function("window.RESULT", timeout=20000)
    res = page.evaluate("window.RESULT")
    b.close()
srv.shutdown()
wasm = np.asarray(Image.open(io.BytesIO(base64.b64decode(res["png"].split(",", 1)[1]))).convert("RGB")).astype(int)
py = Led(8).render(exact).astype(int)
d = np.abs(wasm - py)
print(f"wasm {res['w']}x{res['h']} (frame() -> {res['r']}); python {py.shape[1]}x{py.shape[0]}")
print(f"mean |diff| {d.mean():.3f}  max {d.max()}  pixels off by >2: {(d.max(2) > 2).mean() * 100:.2f}%")
Image.fromarray(np.concatenate([wasm, py], 0).astype(np.uint8)).save(HERE / "out" / "check_led.png")
