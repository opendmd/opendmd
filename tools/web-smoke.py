# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright>=1.45"]
# ///
"""Headless-Chromium smoke test of the management UI, in device mode.

    uv run tools/web-smoke.py                       # against tools/mock-device.py
    uv run tools/web-smoke.py --device http://192.168.1.69   # + a read-mostly real-device pass

Needs `make web` first (the Makefile's web-smoke target does it) and, once,
`uv run --with playwright playwright install chromium`.

Mock pass: starts tools/mock-device.py over the card copy (DMD_CARD_ROOT,
default ~/scratch/dmdos-card) with a fresh overlay under
target/web-smoke/overlay, so the card copy is never written. Then drives
every view like a person would:

  Now       status renders, the preview lights pixels, skip, pause
  Library   open Consoles, search "mario", platform chip SNES, preview one,
            select three
  Sets      build from platforms MD,SNES, save as `smoke` (the file must hold
            exactly the matching filelist.txt lines, in order), activate it,
            delete it (the UI switches back to filelist.txt first)
  Convert   the three selected GIFs -> .odm at their bucketed paths, which
            must be byte-identical to `opendmd-pack convert --layout
            bucketed` on the same GIFs; then regenerate filelist.odm.txt,
            which must equal the card's own
  Settings  PUT brightness, read it back; WiFi scan

Real-device pass (--device): the local build served through
`mock-device.py --proxy`, so nothing is uploaded to the card's www/. Now,
Library (+ a preview), Settings (read only) and one write: save a set named
`smoke` and delete it. No conversion, no filelist changes.

Screenshots of every view at 1280x800 and 390x844 go to target/web-smoke/.
Any console error or page error fails the run.
"""
import argparse
import filecmp
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

from playwright.sync_api import sync_playwright, Page

REPO = Path(__file__).resolve().parents[1]
CARD = Path(os.environ.get("DMD_CARD_ROOT", "~/scratch/dmdos-card"))
OUT = Path(os.environ.get("WEB_SMOKE_OUT", REPO / "target" / "web-smoke"))
SIZES = {"laptop": (1280, 800), "phone": (390, 844)}
VIEWS = ["now", "library", "sets", "convert", "settings"]

failures: list[str] = []


def check(ok: bool, what: str) -> None:
    print(("  ok   " if ok else "  FAIL ") + what)
    if not ok:
        failures.append(what)


def fnv1a32(b: bytes) -> int:
    h = 0x811C9DC5
    for x in b:
        h = ((h ^ x) * 0x01000193) & 0xFFFFFFFF
    return h


def bucketed(gif_path: str) -> str:
    """/gifs/Cat/X.gif -> /odm/Cat/<bb>/X.odm (opendmd_core::layout)."""
    rel = gif_path.split("/", 2)[2]
    d, name = rel.rsplit("/", 1)
    odm = name[:-4] + ".odm"
    return f"/odm/{d}/{fnv1a32(odm.encode()) & 0xFF:02x}/{odm}"


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def start_mock(args: list[str]) -> tuple[subprocess.Popen, str]:
    port = free_port()
    proc = subprocess.Popen(
        ["uv", "run", str(REPO / "tools" / "mock-device.py"), "--port", str(port), "--quiet", *args],
        stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT)
    for _ in range(100):
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                return proc, f"http://127.0.0.1:{port}"
        except OSError:
            time.sleep(0.1)
    proc.kill()
    raise RuntimeError("mock-device.py did not come up")


def get_json(url: str):
    with urllib.request.urlopen(url, timeout=30) as r:
        return json.load(r)


def lit(page: Page, canvas: str) -> int:
    return page.evaluate("""(sel) => {
        const c = document.querySelector(sel);
        const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
        let n = 0;
        for (let i = 0; i < d.length; i += 4) if (d[i] > 60 || d[i+1] > 60 || d[i+2] > 60) n++;
        return n;
    }""", canvas)


def wait_lit(page: Page, canvas: str, what: str, timeout_s: float = 20) -> None:
    deadline = time.time() + timeout_s
    n = 0
    while time.time() < deadline:
        n = lit(page, canvas)
        if n > 200:
            break
        page.wait_for_timeout(250)
    check(n > 200, f"{what}: preview lit ({n} px)")


def shot(page: Page, name: str, size: str) -> None:
    path = OUT / f"{name}-{size}.png"
    page.screenshot(path=str(path))
    print(f"  shot {path}")


def new_page(browser, size: str, errors: list[str]) -> Page:
    w, h = SIZES[size]
    phone = size == "phone"
    ctx = browser.new_context(viewport={"width": w, "height": h}, device_scale_factor=3 if phone else 1,
                              is_mobile=phone, has_touch=phone)
    page = ctx.new_page()
    page.on("console", lambda m: m.type == "error" and errors.append(f"[{size}] console: {m.text}"))
    page.on("pageerror", lambda e: errors.append(f"[{size}] pageerror: {e}"))
    # Chromium's console line for a failed load has no URL; this does.
    page.on("response", lambda r: r.status >= 400 and errors.append(f"[{size}] HTTP {r.status} {r.request.method} {r.url}"))
    page.on("dialog", lambda d: d.accept())
    return page


def goto_view(page: Page, view: str) -> None:
    page.evaluate(f"location.hash = '#{view}'")
    page.wait_for_selector(f"#view-{view}.on")
    page.wait_for_timeout(300)


def open_library(page: Page) -> None:
    goto_view(page, "library")
    page.wait_for_selector("#lib-cat option[value='Consoles']", state="attached", timeout=30000)
    page.select_option("#lib-cat", "Consoles")
    page.wait_for_function("document.querySelector('#lib-count').textContent.includes(' of ')", timeout=60000)
    page.fill("#lib-search", "mario")
    page.wait_for_selector("#lib-chips [data-platform='SNES']")
    page.click("#lib-chips [data-platform='SNES']")
    page.wait_for_timeout(300)
    count = page.inner_text("#lib-count")
    shown = int(count.split(" of ")[0].replace(",", ""))
    check(shown > 0, f"library: Consoles / 'mario' / SNES -> {count}")
    names = page.eval_on_selector_all("#lib-list .vrow:not([hidden]) .t", "els => els.map(e => e.textContent)")
    check(all("mario" in n.lower() for n in names[:shown]), f"library: rows are Mario titles ({names[:3]})")
    page.click("#lib-list .vrow:not([hidden]) >> nth=0")
    wait_lit(page, "#lib-panel", "library")


def build_smoke_set(page: Page) -> int:
    goto_view(page, "sets")
    page.fill("#b-platforms", "MD,SNES")
    page.dispatch_event("#b-platforms", "change")
    page.wait_for_function("/\\d+ match/.test(document.querySelector('#b-count').textContent)", timeout=30000)
    page.click("#b-use")
    page.wait_for_function("/^[1-9]/.test(document.querySelector('#set-count').textContent)")
    page.fill("#set-name", "smoke")
    page.click("#set-save")
    page.wait_for_selector("#sets-list [data-path='/sets/smoke.txt']", timeout=30000)
    return int(page.inner_text("#set-count").split()[0].replace(",", ""))


def expected_md_snes() -> list[str]:
    # The tag parser's platform is the first token (after an optional
    # HALLOWEEN_/XMAS_ season prefix) for these two; that is all this needs.
    lines = [l.rstrip() for l in (CARD / "filelist.txt").read_text(encoding="utf-8-sig").split("\n") if l.strip()]
    pat = re.compile(r"^(?:(?:HALLOWEEN|XMAS)_)?(MD|SNES)_", re.I)
    return [l for l in lines if pat.match(l.rsplit("/", 1)[1])]


def mock_pass(browser) -> None:
    overlay = OUT / "overlay"
    shutil.rmtree(overlay, ignore_errors=True)
    overlay.mkdir(parents=True)
    # Seed the overlay's filelist.odm.txt with every Consoles SNES Mario line
    # pointing back at its GIF, so the converter has something to convert
    # (it skips files the manifest says are done).
    gif_lines = (CARD / "filelist.txt").read_text().split("\n")
    odm_lines = (CARD / "filelist.odm.txt").read_text().split("\n")
    assert len(gif_lines) == len(odm_lines), "filelist.odm.txt is not line-for-line filelist.txt"
    mario = re.compile(r"^/odm/Consoles/[0-9a-f]{2}/SNES_[^/]*mario[^/]*\.odm$", re.I)
    seeded = [g if mario.match(o) else o for g, o in zip(gif_lines, odm_lines)]
    (overlay / "filelist.odm.txt").write_text("\n".join(seeded))
    proc, base = start_mock(["--root", str(CARD), "--overlay", str(overlay), "--dwell", "4"])
    errors: list[str] = []
    try:
        print(f"\n== mock device at {base} (overlay {overlay})")
        page = new_page(browser, "laptop", errors)
        page.goto(base + "/")
        # Now
        page.wait_for_function("document.querySelector('#now-path') && document.querySelector('#now-path').textContent.startsWith('/')", timeout=20000)
        check(page.locator("#now-stats .stat").count() >= 8, "now: status grid renders")
        wait_lit(page, "#now-panel", "now")
        before = page.inner_text("#now-path")
        page.click("#now-skip")
        page.wait_for_function(f"document.querySelector('#now-path').textContent !== {json.dumps(before)}", timeout=10000)
        check(True, "now: skip advanced the device")
        page.click("#now-pause")
        page.wait_for_function("document.querySelector('#now-pause').textContent.startsWith('Resume')")
        check(get_json(base + "/api/status")["paused"], "now: pause reached the device")
        page.click("#now-pause")
        shot(page, "now", "laptop")
        # Library, and three Mario GIFs for the converter
        open_library(page)
        page.eval_on_selector_all("#lib-list .vrow:not([hidden]) input[type=checkbox]",
                                  "els => els.slice(0, 3).forEach(e => e.click())")
        check(page.inner_text("#lib-selbar b") == "3", "library: three selected")
        shot(page, "library", "laptop")
        # Sets
        n = build_smoke_set(page)
        written = overlay / "sets" / "smoke.txt"
        want = expected_md_snes()
        got = written.read_text().split("\n")[:-1] if written.exists() else []
        check(written.exists() and got == want, f"sets: sets/smoke.txt holds the {len(want)} MD/SNES lines in order (got {len(got)}, editor {n})")
        page.click("#sets-list [data-path='/sets/smoke.txt'] >> text=Activate")
        page.wait_for_function("document.querySelector('#set-active').textContent.includes('smoke')", timeout=60000)
        st = get_json(base + "/api/status")
        check(get_json(base + "/api/config")["set"] == "smoke" and st["manifest"] == "/sdcard/sets/smoke.txt",
              f"sets: activated ({st['entries']} entries)")
        shot(page, "sets", "laptop")
        page.click("#sets-list [data-path='/sets/smoke.txt'] >> text=Delete")
        page.wait_for_selector("#sets-list [data-path='/sets/smoke.txt']", state="detached", timeout=60000)
        check(not written.exists() and get_json(base + "/api/config")["set"] == "",
              "sets: deleted after switching back to filelist.txt")
        # Convert the three selected GIFs
        goto_view(page, "library")
        page.click("#lib-to-convert")
        page.wait_for_selector("#view-convert.on")
        page.wait_for_function("document.querySelector('#cv-progress').textContent.includes('3 queued')", timeout=10000)
        # The overlay's filelist.odm.txt forgets these three, so they are
        # converted rather than skipped.
        page.click("#cv-start")
        page.wait_for_function("document.querySelector('#cv-progress').textContent.startsWith('3 converted')", timeout=120000)
        new = sorted(p for p in (overlay / "odm").rglob("*.odm"))
        check(len(new) == 3, f"convert: 3 .odm files uploaded ({[str(p.relative_to(overlay)) for p in new]})")
        compare_with_pack(overlay, new)
        page.click("#cv-manifest-write")
        page.wait_for_timeout(1500)
        regenerated = overlay / "filelist.odm.txt"
        done = {"/" + str(p.relative_to(overlay)) for p in new}
        want = "\n".join(o if o in done else s_ for s_, o in zip(seeded, odm_lines))
        check(regenerated.exists() and regenerated.read_text() == want,
              "convert: regenerated filelist.odm.txt = the seeded one with exactly the 3 new lines switched to .odm")
        shot(page, "convert", "laptop")
        # Settings
        goto_view(page, "settings")
        page.wait_for_selector("#st-facts .stat")
        page.eval_on_selector("#st-brightness", "e => { e.value = 120; e.dispatchEvent(new Event('input')); }")
        page.click("#st-save-play")
        page.wait_for_timeout(800)
        check(get_json(base + "/api/config")["brightness"] == 120, "settings: brightness PUT and read back")
        page.click("#st-scan")
        page.wait_for_selector("#st-nets .chip", timeout=15000)
        check(True, "settings: WiFi scan listed networks")
        shot(page, "settings", "laptop")
        phone_pass(browser, base, errors, convert=True)
    finally:
        proc.terminate()
    report_errors("mock", errors)


def compare_with_pack(overlay: Path, new: list[Path]) -> None:
    """Run opendmd-pack on the same three GIFs and compare bytes."""
    mini = OUT / "pack-ref"
    shutil.rmtree(mini, ignore_errors=True)
    for odm in new:
        rel = odm.relative_to(overlay / "odm")  # Cat/bb/name.odm
        cat = str(rel.parent.parent)
        gif = CARD / "gifs" / cat / (odm.stem + ".gif")
        (mini / "gifs" / cat).mkdir(parents=True, exist_ok=True)
        shutil.copy(gif, mini / "gifs" / cat / gif.name)
    subprocess.run(["cargo", "run", "--release", "--quiet", "--", "convert", str(mini), "--layout", "bucketed", "--verify"],
                   cwd=REPO / "opendmd-pack", check=True, env={**os.environ, "RUSTUP_TOOLCHAIN": "stable"},
                   stdout=subprocess.DEVNULL)
    for odm in new:
        ref = mini / odm.relative_to(overlay)
        same = ref.exists() and filecmp.cmp(ref, odm, shallow=False)
        check(same, f"convert: {odm.relative_to(overlay)} byte-identical to opendmd-pack ({odm.stat().st_size} B, magic {odm.read_bytes()[:4]!r})")


def phone_pass(browser, base: str, errors: list[str], convert: bool) -> None:
    page = new_page(browser, "phone", errors)
    page.goto(base + "/")
    page.wait_for_function("document.querySelector('#now-path') && document.querySelector('#now-path').textContent.startsWith('/')", timeout=20000)
    wait_lit(page, "#now-panel", "phone now")
    shot(page, "now", "phone")
    for view in ["library", "sets"] + (["convert"] if convert else []) + ["settings"]:
        goto_view(page, view)
        page.wait_for_timeout(1500)
        shot(page, view, "phone")
    overflow = page.evaluate("document.documentElement.scrollWidth - document.documentElement.clientWidth")
    check(overflow <= 0, f"phone: no horizontal scroll ({overflow}px)")
    page.context.close()


def device_pass(browser, device: str) -> None:
    proc, base = start_mock(["--proxy", device])
    errors: list[str] = []
    try:
        print(f"\n== real device {device} via local proxy {base} (read-mostly)")
        page = new_page(browser, "laptop", errors)
        page.goto(base + "/")
        page.wait_for_function("document.querySelector('#now-path') && document.querySelector('#now-path').textContent.startsWith('/')", timeout=30000)
        check(page.locator("#now-stats .stat").count() >= 8, "device now: status grid renders")
        wait_lit(page, "#now-panel", "device now", 40)
        shot(page, "device-now", "laptop")
        open_library(page)
        shot(page, "device-library", "laptop")
        goto_view(page, "settings")
        page.wait_for_selector("#st-facts .stat", timeout=20000)
        check(page.input_value("#st-hostname") != "", "device settings: config GET rendered")
        shot(page, "device-settings", "laptop")
        # The one write: save and delete a set named smoke (never activated).
        n = build_smoke_set(page)
        with urllib.request.urlopen(base + "/sets/smoke.txt", timeout=30) as r:
            body = r.read().decode()
        check(len(body.split("\n")) - 1 == n and body.endswith("\n"), f"device sets: /sets/smoke.txt saved ({n} lines)")
        shot(page, "device-sets", "laptop")
        page.click("#sets-list [data-path='/sets/smoke.txt'] >> text=Delete")
        page.wait_for_selector("#sets-list [data-path='/sets/smoke.txt']", state="detached", timeout=30000)
        listing = get_json(base + "/api/files?dir=/sets")
        check(not any(e["name"].startswith("smoke.txt") for e in listing["entries"]), "device sets: smoke deleted")
        page.context.close()
        phone = new_page(browser, "phone", errors)
        phone.goto(base + "/")
        phone.wait_for_function("document.querySelector('#now-path') && document.querySelector('#now-path').textContent.startsWith('/')", timeout=30000)
        wait_lit(phone, "#now-panel", "device phone now", 40)
        shot(phone, "device-now", "phone")
        goto_view(phone, "settings")
        phone.wait_for_selector("#st-facts .stat", timeout=20000)
        shot(phone, "device-settings", "phone")
        phone.context.close()
        st = get_json(base + "/api/status")
        print(f"  device after: {st['now_playing']['path']} heap free {st['heap']['free']} floor {st['heap']['floor']} "
              f"misses {st['deadline_misses']} worst {st['worst_lateness_ms']} ms")
    finally:
        proc.terminate()
    report_errors("device", errors)


def report_errors(name: str, errors: list[str]) -> None:
    check(not errors, f"{name}: zero console/page errors")
    for e in errors:
        print("  !!   " + e)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--device", help="also run the read-mostly pass against this device URL")
    ap.add_argument("--device-only", action="store_true")
    args = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True)
        if not args.device_only:
            mock_pass(browser)
        if args.device:
            device_pass(browser, args.device.rstrip("/"))
        browser.close()
    print(f"\n{'FAILED: ' + str(len(failures)) if failures else 'all checks passed'}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
