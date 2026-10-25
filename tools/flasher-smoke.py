# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright>=1.45"]
# ///
"""Headless-Chromium check of the browser flasher (flasher/index.html).

    uv run tools/flasher-smoke.py            # needs dist/ from `make fw-release`

Lays the site out the way `make flasher` does (target/flasher-smoke/:
index.html, firmware/ with dist/'s two manifests, one per flash layout, and
the files they name), serves it on a free localhost port, and checks: ESP
Web Tools loads from jsDelivr (the pinned version) and defines
`<esp-web-install-button>`; both Install buttons (DevKit 4 MB, OpenDMD
board 8 MB) render (headless Chromium has `navigator.serial`, so the
"unsupported" slot must not be what shows); each manifest parses, carries
its name ("OpenDMD", "OpenDMD 8 MB": the firmware's Improv answer), names
six parts at its layout's offsets (4mb: 0x1000/0x8000/0xF000/0x20000/
0x260000/0x2C0000; 8mb: the pack and the updater first, at 0x380000 and
0x320000, then 0x1000/0x8000/0xF000/0x20000; docs/ota.md), each one is
served, and the page lists them; no part runs into the next and each set
ends inside its flash; the footer links the third-party notices, which are
served; no console or page errors.

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
# The third-party notices fw-release puts in dist/, linked from the page's footer.
NOTICES = ("THIRD-PARTY-LICENSES-firmware.txt", "THIRD-PARTY-LICENSES-web.txt")


def check(ok: bool, what: str) -> None:
    global failures
    print(("  ok   " if ok else "  FAIL ") + what)
    failures += not ok


# Per flash layout: the manifest's name, its parts' offsets in the order
# written, and the flash size.
LAYOUTS = {
    "4mb": ("OpenDMD", [0x1000, 0x8000, 0xF000, 0x20000, 0x260000, 0x2C0000], 4 << 20),
    "8mb": ("OpenDMD 8 MB", [0x380000, 0x320000, 0x1000, 0x8000, 0xF000, 0x20000], 8 << 20),
}


def main() -> int:
    manifests = {v: DIST / f"manifest-{v}.json" for v in LAYOUTS}
    if not all(p.exists() for p in manifests.values()):
        print("no dist/manifest-4mb.json and manifest-8mb.json: run make fw-release first")
        return 2
    ms = {v: json.loads(p.read_text()) for v, p in manifests.items()}
    shutil.rmtree(SITE, ignore_errors=True)
    (SITE / "firmware").mkdir(parents=True)
    shutil.copy(REPO / "flasher" / "index.html", SITE / "index.html")
    all_parts: dict[str, list] = {}
    for v, m in ms.items():
        shutil.copy(manifests[v], SITE / "firmware" / manifests[v].name)
        parts = all_parts[v] = m["builds"][0]["parts"]
        for part in parts:
            shutil.copy(DIST / part["path"], SITE / "firmware" / part["path"])
        name, want, flash = LAYOUTS[v]
        offsets = [p["offset"] for p in parts]
        check(m["builds"][0]["chipFamily"] == "ESP32" and offsets == want,
              f"manifest-{v}: ESP32, parts at {[hex(o) for o in offsets]}")
        check(m.get("name") == name and m.get("version") == ms["4mb"]["version"], f"manifest-{v}: {m.get('name')!r} {m.get('version')}")
        check(m.get("new_install_prompt_erase") is True, f"manifest-{v}: the installer asks whether to erase")
        spans = sorted((p["offset"], p["offset"] + (DIST / p["path"]).stat().st_size) for p in parts)
        check(all(e <= n for (_, e), (n, _) in zip(spans, spans[1:] + [(flash, 0)])),
              f"manifest-{v}: no part runs into the next, all inside {flash >> 20} MB (ends {[hex(e) for _, e in spans]})")
        check(any(p["path"] == f"bootloader-{v}.bin" for p in parts) and any(p["path"] == f"partition-table-{v}.bin" for p in parts),
              f"manifest-{v}: its own bootloader and partition table")
    for name in NOTICES:
        shutil.copy(DIST / name, SITE / "firmware" / name)

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
        page.wait_for_timeout(1500)
        for v, m in ms.items():
            visible = page.locator(f"#install-activate-{v}").is_visible()
            unsupported = page.evaluate(f"document.querySelector('#install-{v}').hasAttribute('install-unsupported')")
            manifest_attr = page.get_attribute(f"#install-{v}", "manifest")
            label = page.inner_text(f"#install-activate-{v}") if visible else ""
            check(visible and not unsupported and manifest_attr == f"firmware/manifest-{v}.json",
                  f"the {v} Install button renders ({label!r}, {manifest_attr}; visible {visible}, unsupported {unsupported})")
            page.wait_for_function(f"document.querySelectorAll('#parts-{v} tr').length === 6", timeout=10000)
            rows = page.eval_on_selector_all(f"#parts-{v} tr", "rs => rs.map(r => r.innerText.replace(/\\s+/g, ' ').trim())")
            last = hex(LAYOUTS[v][1][-1]).upper().replace("0X", "0x")
            check(len(rows) == 6 and rows[-1].endswith(last), f"the page lists the {v} parts: {rows}")
            check(page.inner_text(f"#version-{v}") == f"{m['name']} {m['version']}", f"the page shows {page.inner_text(f'#version-{v}')!r}")
            for part in all_parts[v]:
                r = page.request.get(f"{base}/firmware/{part['path']}")
                check(r.status == 200 and len(r.body()) == (DIST / part["path"]).stat().st_size,
                      f"served: firmware/{part['path']} ({len(r.body()):,} B)")
        check(page.inner_text("#board-4mb h2") == "DevKit (4 MB)" and page.inner_text("#board-8mb h2") == "8 MB build",
              "the two choices are named for the boards")
        hrefs = page.eval_on_selector_all("footer a", "as => as.map(a => a.getAttribute('href'))")
        for name in NOTICES:
            r = page.request.get(f"{base}/firmware/{name}")
            check(f"firmware/{name}" in hrefs and r.status == 200 and b"Third-party licences" in r.body(),
                  f"the footer links firmware/{name}, served ({len(r.body()):,} B)")
        text = page.inner_text("main")
        check("Erase device" in text and "Chrome, Edge" in text and "NVS" in text
              and "Change Wi-Fi" in text and "Not sure which you have?" in text,
              "the page explains the browser, the erase, what it takes, the WiFi step and which board")
        page.screenshot(path=str(SITE / "flasher.png"), full_page=True)
        browser.close()
    srv.shutdown()
    check(not errors, f"zero console/page errors {errors}")
    print(f"\n{'FAILED: ' + str(failures) if failures else 'all checks passed'} (screenshot: {SITE / 'flasher.png'})")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
