# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright>=1.45"]
# ///
"""Headless-browser smoke test for the OpenDMD web app.

Usage:
    uv run tools/web-smoke.py

Serves opendmd-web/www over a local http.server (run from anywhere; needs
`uv run --with playwright playwright install chromium` once), drives it with headless
Chromium through the same controls a person would use (file input, platform
filter, play/next/pause, sliders, flat toggle), and reports console errors,
status-line readings, screenshots, an FPS estimate and a lit-pixel count.

Does not modify opendmd-web/src/ or www/index.html/app.js: if the page is
broken this script reports how, it does not work around it.
"""
import http.server
import json
import socket
import subprocess
import sys
import time
from pathlib import Path

from playwright.sync_api import sync_playwright

import os

REPO = Path(__file__).resolve().parents[1]  # repository root
WWW = REPO / "opendmd-web" / "www"
# A card root (the directory holding gifs/); override with DMD_CARD_ROOT.
GIF_ROOT = Path(os.environ.get("DMD_CARD_ROOT", "~/scratch/dmdos-card")) / "gifs"
# Where screenshots go; override with WEB_SMOKE_OUT.
SCRATCHPAD = Path(os.environ.get("WEB_SMOKE_OUT", REPO / "target" / "web-smoke"))
SCRATCHPAD.mkdir(parents=True, exist_ok=True)


def free_port() -> int:
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def pick_files() -> list[Path]:
    consoles = GIF_ROOT / "Consoles"
    pinball = GIF_ROOT / "Pinball_Short"
    snes = sorted(p for p in consoles.glob("SNES_*.gif"))[:4]
    md = sorted(p for p in consoles.glob("MD_*.gif"))[:4]
    pb = sorted(p for p in pinball.glob("*.gif"))[:4]
    assert len(snes) == 4, f"expected 4 SNES files, got {len(snes)}"
    assert len(md) == 4, f"expected 4 MD files, got {len(md)}"
    assert len(pb) == 4, f"expected 4 Pinball_Short files, got {len(pb)}"
    return snes + md + pb


def main() -> None:
    SCRATCHPAD.mkdir(parents=True, exist_ok=True)
    port = free_port()

    server = subprocess.Popen(
        ["uv", "run", "python", "-m", "http.server", str(port), "--directory", str(WWW)],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    try:
        # Wait for the server to accept connections.
        for _ in range(50):
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                    break
            except OSError:
                time.sleep(0.1)
        else:
            raise RuntimeError("http.server did not come up")

        files = pick_files()
        print(f"Selected {len(files)} files:")
        for f in files:
            print(f"  {f}")

        console_messages: list[dict] = []
        page_errors: list[str] = []

        with sync_playwright() as p:
            browser = p.chromium.launch(headless=True)
            page = browser.new_page(viewport={"width": 1400, "height": 900})

            page.on(
                "console",
                lambda msg: console_messages.append(
                    {"type": msg.type, "text": msg.text}
                ),
            )
            page.on("pageerror", lambda exc: page_errors.append(str(exc)))

            url = f"http://127.0.0.1:{port}/index.html"
            print(f"\nLoading {url}")
            response = page.goto(url, wait_until="load")
            print(f"HTTP status: {response.status if response else 'no response'}")

            # Give the wasm module a moment to init.
            page.wait_for_timeout(500)

            report: dict = {"console": [], "page_errors": [], "status_lines": {}}

            # --- Select 12 GIF files through the plain file input ---
            file_input = page.locator("#files")
            file_input.set_input_files([str(f) for f in files])
            page.wait_for_timeout(300)
            status_after_pick = page.locator("#status").inner_text()
            print(f"\nStatus after file pick: {status_after_pick!r}")

            play_disabled = file_input.evaluate("el => document.getElementById('play').disabled")
            print(f"Play button disabled after pick: {play_disabled}")

            # --- Leave platform filter empty, start playback ---
            play_btn = page.locator("#play")
            play_btn.click()
            page.wait_for_timeout(2000)

            status1 = page.locator("#status").inner_text()
            print(f"\nStatus after play (no filter), +2s: {status1!r}")
            report["status_lines"]["1_no_filter"] = status1

            shot1 = SCRATCHPAD / "web-smoke-1.png"
            page.screenshot(path=str(shot1))
            canvas_shot1 = SCRATCHPAD / "web-smoke-canvas-1.png"
            page.locator("#panel").screenshot(path=str(canvas_shot1))
            print(f"Screenshots: {shot1}, {canvas_shot1}")

            # --- Set platforms to MD,SNES, restart ---
            platforms_input = page.locator("#platforms")
            platforms_input.fill("MD,SNES")
            play_btn.click()  # re-press play to rebuild with the new filter
            page.wait_for_timeout(2000)

            status2 = page.locator("#status").inner_text()
            print(f"\nStatus after platform filter MD,SNES, +2s: {status2!r}")
            report["status_lines"]["2_md_snes_filter"] = status2

            shot2 = SCRATCHPAD / "web-smoke-2.png"
            page.screenshot(path=str(shot2))
            print(f"Screenshot: {shot2}")

            # --- Skip 3 times with 500ms gaps ---
            next_btn = page.locator("#next")
            skip_statuses = []
            for i in range(3):
                next_btn.click()
                page.wait_for_timeout(500)
                s = page.locator("#status").inner_text()
                skip_statuses.append(s)
                print(f"Status after skip {i + 1}: {s!r}")
            report["status_lines"]["3_skips"] = skip_statuses

            # --- Toggle pause, wait, confirm frame counter stops ---
            pause_btn = page.locator("#pause")
            pause_btn.click()  # pause
            page.wait_for_timeout(200)
            status_paused_1 = page.locator("#status").inner_text()
            page.wait_for_timeout(1000)
            status_paused_2 = page.locator("#status").inner_text()
            print(f"\nStatus right after pause: {status_paused_1!r}")
            print(f"Status 1s later (still paused): {status_paused_2!r}")
            report["status_lines"]["4_paused_immediately"] = status_paused_1
            report["status_lines"]["4_paused_after_1s"] = status_paused_2
            pause_btn.click()  # unpause
            page.wait_for_timeout(200)

            # --- Bloom to max, gain to ~1.5 ---
            bloom_slider = page.locator("#bloom")
            gain_slider = page.locator("#gain")
            bloom_max = bloom_slider.evaluate("el => el.max")
            bloom_slider.fill(str(bloom_max))
            bloom_slider.dispatch_event("input")
            gain_slider.fill("1.5")
            gain_slider.dispatch_event("input")
            page.wait_for_timeout(1000)

            bloom_out = page.locator("#bloom-out").inner_text()
            gain_out = page.locator("#gain-out").inner_text()
            print(f"\nBloom output reads: {bloom_out!r}, gain output reads: {gain_out!r}")

            shot3 = SCRATCHPAD / "web-smoke-3.png"
            page.screenshot(path=str(shot3))
            print(f"Screenshot: {shot3}")

            # --- Toggle flat mode ---
            flat_checkbox = page.locator("#flat")
            flat_checkbox.check()
            page.wait_for_timeout(300)

            shot4 = SCRATCHPAD / "web-smoke-4.png"
            page.screenshot(path=str(shot4))
            print(f"Screenshot: {shot4}")

            # --- Measure FPS over 3 seconds via rAF tick counting ---
            fps = page.evaluate(
                """
                () => new Promise((resolve) => {
                    let ticks = 0;
                    const start = performance.now();
                    function tick() {
                        ticks++;
                        if (performance.now() - start < 3000) {
                            requestAnimationFrame(tick);
                        } else {
                            resolve(ticks / ((performance.now() - start) / 1000));
                        }
                    }
                    requestAnimationFrame(tick);
                })
                """
            )
            print(f"\nMeasured rAF FPS over ~3s: {fps:.2f}")
            report["fps_raf"] = fps

            final_status = page.locator("#status").inner_text()
            print(f"Final status line: {final_status!r}")
            report["status_lines"]["5_final"] = final_status

            # --- Lit-pixel assertion on the canvas ---
            lit_count = page.evaluate(
                """
                () => {
                    const canvas = document.getElementById('panel');
                    const ctx = canvas.getContext('2d');
                    const { data } = ctx.getImageData(0, 0, canvas.width, canvas.height);
                    let lit = 0;
                    for (let i = 0; i < data.length; i += 4) {
                        if (data[i] > 40 || data[i + 1] > 40 || data[i + 2] > 40) {
                            lit++;
                        }
                    }
                    return lit;
                }
                """
            )
            total_pixels = page.evaluate(
                "() => { const c = document.getElementById('panel'); return c.width * c.height; }"
            )
            print(f"\nLit pixels (>40 in any channel): {lit_count} / {total_pixels}")
            report["lit_pixels"] = lit_count
            report["total_pixels"] = total_pixels

            browser.close()

            report["console"] = console_messages
            report["page_errors"] = page_errors

            print("\n--- Console messages ---")
            if not console_messages:
                print("(none)")
            for m in console_messages:
                print(f"[{m['type']}] {m['text']}")

            print("\n--- Page errors ---")
            if not page_errors:
                print("(none)")
            for e in page_errors:
                print(e)

            report_path = SCRATCHPAD / "web-smoke-report.json"
            report_path.write_text(json.dumps(report, indent=2))
            print(f"\nFull JSON report: {report_path}")

    finally:
        server.terminate()
        try:
            server.wait(timeout=5)
        except subprocess.TimeoutExpired:
            server.kill()


if __name__ == "__main__":
    main()
