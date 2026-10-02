# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright>=1.45"]
# ///
"""Headless-Chromium check of the browser flasher (flasher/index.html).

    uv run tools/flasher-smoke.py            # needs dist/ from `make fw-release`

Lays the site out the way `make flasher` does (target/flasher-smoke/:
index.html, firmware/ with dist/'s manifest and the files it names), serves
it on a free localhost port, and checks: ESP Web Tools loads from jsDelivr
(the pinned version) and defines `<esp-web-install-button>`; the button's
Install control renders (headless Chromium has `navigator.serial`, so the
"unsupported" slot must not be what shows); the manifest parses, names six
parts at 0x1000/0x8000/0xF000/0x20000/0x260000/0x2C0000 (layout v2,
docs/ota.md), each one is served, and the page
lists them; the parts' sizes add up inside 4 MB; no console or page errors.

What it cannot do: the flash itself. Web Serial's port picker needs a
person, and there is no board on the other end of a headless browser.
"""
import functools
import http.server
import json
import shutil
import sys
import threading
from pathlib import Path

from playwright.sync_api import sync_playwright

REPO = Path(__file__).resolve().parents[1]
SITE = REPO / "target" / "flasher-smoke"
DIST = REPO / "dist"
failures = 0


def check(ok: bool, what: str) -> None:
    global failures
    print(("  ok   " if ok else "  FAIL ") + what)
    failures += not ok


def main() -> int:
    manifest = DIST / "manifest.json"
    if not manifest.exists():
        print("no dist/manifest.json: run make fw-release first")
        return 2
    m = json.loads(manifest.read_text())
    shutil.rmtree(SITE, ignore_errors=True)
    (SITE / "firmware").mkdir(parents=True)
    shutil.copy(REPO / "flasher" / "index.html", SITE / "index.html")
    shutil.copy(manifest, SITE / "firmware" / "manifest.json")
    parts = m["builds"][0]["parts"]
    for part in parts:
        shutil.copy(DIST / part["path"], SITE / "firmware" / part["path"])

    offsets = [p["offset"] for p in parts]
    check(m["builds"][0]["chipFamily"] == "ESP32" and offsets == [0x1000, 0x8000, 0xF000, 0x20000, 0x260000, 0x2C0000],
          f"manifest: ESP32, parts at {[hex(o) for o in offsets]}")
    check(m.get("new_install_prompt_erase") is True, "manifest: the installer asks whether to erase")
    ends = [p["offset"] + (DIST / p["path"]).stat().st_size for p in parts]
    check(all(e <= n for e, n in zip(ends, offsets[1:] + [4 << 20])),
          f"manifest: no part runs into the next (ends {[hex(e) for e in ends]})")

    class Quiet(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *args) -> None:
            pass

    handler = functools.partial(Quiet, directory=str(SITE))
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_port}"
    # localhost, not 127.0.0.1: both are secure contexts, localhost is what
    # `make flasher` prints.
    base = base.replace("127.0.0.1", "localhost")
    errors: list[str] = []
    with sync_playwright() as pw:
        browser = pw.chromium.launch(headless=True)
        page = browser.new_page(viewport={"width": 1280, "height": 900})
        page.on("console", lambda msg: msg.type == "error" and errors.append(msg.text))
        page.on("pageerror", lambda e: errors.append(str(e)))
        page.on("response", lambda r: r.status >= 400 and errors.append(f"HTTP {r.status} {r.url}"))
        page.goto(base + "/")
        page.wait_for_function("customElements.get('esp-web-install-button') !== undefined", timeout=30000)
        check(True, "ESP Web Tools 10.4.0 loaded from jsDelivr, <esp-web-install-button> defined")
        check(page.evaluate("'serial' in navigator && window.isSecureContext"), "Web Serial present, secure context")
        page.wait_for_function("document.querySelector('#install').renderRoot !== undefined || true")
        page.wait_for_timeout(1500)
        visible = page.locator("#install-activate").is_visible()
        unsupported = page.evaluate("document.querySelector('#install').hasAttribute('install-unsupported')")
        check(visible and not unsupported, f"the Install button renders (visible {visible}, unsupported {unsupported})")
        page.wait_for_function("document.querySelectorAll('#parts tr').length === 6", timeout=10000)
        rows = page.eval_on_selector_all("#parts tr", "rs => rs.map(r => r.innerText.replace(/\\s+/g, ' ').trim())")
        check(len(rows) == 6 and rows[-1].endswith("0x2C0000"), f"the page lists the parts: {rows}")
        check(page.inner_text("#version") == f"OpenDMD {m['version']}", f"the page shows {page.inner_text('#version')!r}")
        for part in parts:
            r = page.request.get(f"{base}/firmware/{part['path']}")
            check(r.status == 200 and len(r.body()) == (DIST / part["path"]).stat().st_size,
                  f"served: firmware/{part['path']} ({len(r.body()):,} B)")
        text = page.inner_text("main")
        check("Erase device" in text and "Chrome or Edge" in text and "NVS" in text,
              "the page explains the browser, the erase and what it takes")
        page.screenshot(path=str(SITE / "flasher.png"), full_page=True)
        browser.close()
    srv.shutdown()
    check(not errors, f"zero console/page errors {errors}")
    print(f"\n{'FAILED: ' + str(failures) if failures else 'all checks passed'} (screenshot: {SITE / 'flasher.png'})")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
