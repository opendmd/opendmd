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

  Now       status renders, the preview lights pixels, skip, pause; "Send
            to panel" of a card path (POST /api/play) shows in status, and a
            missing path / a non-GIF give the 404 / 415 toasts
  Library   open Consoles, search "mario", platform chip SNES, preview one,
            send it to the panel, select three
  Sets      the root manifests are listed with the card default marked
            (filelist.odm.txt, from status `manifest`); activate
            filelist.xmas by name and the default back as ""; build from
            platforms MD,SNES, save as `smoke` (the file must hold exactly
            the matching filelist.txt lines, in order), activate it, delete
            it (the UI switches back to the default first)
  Convert   the three selected GIFs -> .odm at their bucketed paths, which
            must be byte-identical to `opendmd-pack convert --layout
            bucketed` on the same GIFs; then regenerate filelist.odm.txt,
            which must equal the card's own. Then, with the page told there
            is no filelist.odm.txt, the same three again: skipped after one
            GET /api/stat each and no bucket listing
  Settings  www_version and index_state shown; PUT brightness, read it
            back; WiFi scan; with /api/status rewritten to index_state
            "building" (Playwright route), set switching is disabled in
            Settings and Sets, and comes back when it reads "ready"

A few requests fail on purpose (the 404/415 plays); they are listed in
`expected` and are not counted as console errors.

Real-device pass (--device): the local build served through
`mock-device.py --proxy`, so nothing is uploaded to the card's www/. Now,
Library (+ a preview, sent to the panel with POST /api/play, which must show
up in /api/status), Settings (read only) and one write: save a set named
`smoke` and delete it. No conversion, no filelist or config changes.

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


# URL substrings of requests that fail on purpose (a 404/415 play).
expected: list[str] = []


def unexpected(url: str) -> bool:
    return not any(e in url for e in expected)


def new_page(browser, size: str, errors: list[str]) -> Page:
    w, h = SIZES[size]
    phone = size == "phone"
    ctx = browser.new_context(viewport={"width": w, "height": h}, device_scale_factor=3 if phone else 1,
                              is_mobile=phone, has_touch=phone)
    page = ctx.new_page()
    page.on("console", lambda m: m.type == "error" and unexpected(m.location.get("url", ""))
            and errors.append(f"[{size}] console: {m.text}"))
    page.on("pageerror", lambda e: errors.append(f"[{size}] pageerror: {e}"))
    # Chromium's console line for a failed load has no URL; this does.
    page.on("response", lambda r: r.status >= 400 and unexpected(r.url)
            and errors.append(f"[{size}] HTTP {r.status} {r.request.method} {r.url}"))
    page.on("dialog", lambda d: d.accept())
    return page


def goto_view(page: Page, view: str) -> None:
    page.evaluate(f"location.hash = '#{view}'")
    page.wait_for_selector(f"#view-{view}.on")
    page.wait_for_timeout(300)


def wait_playing(base: str, path: str, timeout_s: float = 6) -> str:
    """Poll /api/status until now_playing.path is `path`; returns the last one."""
    deadline = time.time() + timeout_s
    cur = ""
    while time.time() < deadline:
        cur = get_json(base + "/api/status")["now_playing"]["path"]
        if cur == path:
            break
        time.sleep(0.2)
    return cur


def last_toast(page: Page, kind: str) -> str:
    page.wait_for_selector(f"#toasts .toast.{kind}", timeout=10000)
    return page.eval_on_selector_all(f"#toasts .toast.{kind}", "els => els.at(-1).textContent")


def send_from_library(page: Page, base: str, what: str) -> None:
    page.wait_for_selector("#lib-play:not([disabled])", timeout=10000)
    path = page.get_attribute("#lib-play", "title")
    page.click("#lib-play")
    cur = wait_playing(base, path)
    check(cur == path, f"{what}: Send to panel -> /api/status plays {path} (got {cur})")


def root_manifest_rows(page: Page) -> list[str]:
    return page.eval_on_selector_all("#sets-list [data-path^='/filelist']", "els => els.map(e => e.dataset.path)")


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
        page.wait_for_function("document.querySelector('#now-pause').textContent.startsWith('Pause')")
        # Send to panel: a real GIF, then a miss (404) and a manifest (415).
        gif = next(l for l in (CARD / "filelist.txt").read_text(encoding="utf-8-sig").split("\n") if l.strip().endswith(".gif")).strip()
        page.fill("#now-play-path", gif)
        page.click("#now-play")
        cur = wait_playing(base, gif)
        check(cur == gif, f"now: Send to panel {gif} -> /api/status (got {cur})")
        page.wait_for_function(f"document.querySelector('#now-path').textContent === {json.dumps(gif)}", timeout=10000)
        expected.append("/api/play")
        page.fill("#now-play-path", "/gifs/Consoles/no-such-file.gif")
        page.click("#now-play")
        t = last_toast(page, "bad")
        check("not on the card" in t, f"now: 404 play -> toast '{t}'")
        page.evaluate("document.querySelectorAll('#toasts .toast').forEach(t => t.remove())")
        page.fill("#now-play-path", "/filelist.txt")
        page.click("#now-play")
        t = last_toast(page, "bad")
        check("not a GIF or .odm" in t, f"now: 415 play -> toast '{t}'")
        shot(page, "now", "laptop")
        page.evaluate("document.querySelectorAll('#toasts .toast').forEach(t => t.remove())")
        expected.remove("/api/play")
        # Library, and three Mario GIFs for the converter
        open_library(page)
        send_from_library(page, base, "library")
        page.eval_on_selector_all("#lib-list .vrow:not([hidden]) input[type=checkbox]",
                                  "els => els.slice(0, 3).forEach(e => e.click())")
        check(page.inner_text("#lib-selbar b") == "3", "library: three selected")
        shot(page, "library", "laptop")
        # Sets: root manifests, the default, activation by file name
        goto_view(page, "sets")
        page.wait_for_selector("#sets-list [data-path='/filelist.xmas']", timeout=20000)
        rows = root_manifest_rows(page)
        check({"/filelist.txt", "/filelist.odm.txt", "/filelist.hall", "/filelist.xmas"} <= set(rows),
              f"sets: root manifests listed ({rows})")
        dflt = page.eval_on_selector_all("#sets-list [data-path] .tag:not(.act)", "els => els.filter(e => e.textContent === 'default').map(e => e.closest('[data-path]').dataset.path)")
        check(dflt == ["/filelist.odm.txt"], f"sets: default marked on {dflt} (status manifest {get_json(base + '/api/status')['manifest']})")
        check("(default)" in page.inner_text("#set-active") and "not possible" not in page.inner_text("#view-sets"),
              f"sets: '{page.inner_text('#set-active')}', no stale note")
        page.click("#sets-list [data-path='/filelist.xmas'] >> text=Activate")
        page.wait_for_function("document.querySelector('#set-active').textContent.includes('filelist.xmas')", timeout=60000)
        st, cf = get_json(base + "/api/status"), get_json(base + "/api/config")
        check(cf["set"] == "filelist.xmas" and st["manifest"] == "/sdcard/filelist.xmas",
              f"sets: filelist.xmas activated by name ({st['entries']} entries)")
        shot(page, "sets-root", "laptop")
        page.click("#sets-list [data-path='/filelist.odm.txt'] >> text=Activate")
        page.wait_for_function("document.querySelector('#set-active').textContent.includes('(default)')", timeout=60000)
        st, cf = get_json(base + "/api/status"), get_json(base + "/api/config")
        check(cf["set"] == "" and st["manifest"] == "/sdcard/filelist.odm.txt", "sets: the default activated as set \"\"")
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
              "sets: deleted after switching back to the default")
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
        tip = page.get_attribute("label:has(#cv-skip)", "title") or ""
        check("stat" in tip and "gifs/Consoles" in tip, "convert: the skip checkbox explains the stat cost")
        # Without filelist.odm.txt the skip check is one /api/stat per file.
        page.evaluate("async () => { const { app } = await import('./js/state.js'); app.odm = new Set(); app.odmSource = 'none'; }")
        page.click("text=Clear finished")
        reqs: list[str] = []
        page.on("request", lambda r: reqs.append(r.url))
        goto_view(page, "library")
        page.click("#lib-to-convert")
        page.wait_for_function("document.querySelector('#cv-progress').textContent.includes('3 queued')", timeout=10000)
        page.click("#cv-start")
        page.wait_for_function("document.querySelector('#cv-progress').textContent.includes('3 skipped')", timeout=60000)
        stats = [u for u in reqs if "/api/stat?" in u]
        lists = [u for u in reqs if "/api/files?dir=%2Fodm" in u]
        check(len(stats) == 3 and not lists and all("%2Fodm%2FConsoles%2F" in u for u in stats),
              f"convert: no manifest -> 3 skipped after {len(stats)} /api/stat of the bucketed .odm, {len(lists)} listings")
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
        version = (REPO / "opendmd-web" / "www" / "version.txt").read_text().strip()
        facts = page.inner_text("#st-facts")
        check(version in facts and "ready" in facts, f"settings: www_version {version} and index_state shown")
        opts = page.eval_on_selector_all("#st-set option", "els => els.map(e => e.value)")
        check(opts[0] == "" and "filelist.xmas" in opts, f"settings: set choices include the root manifests ({opts[:5]})")
        shot(page, "settings", "laptop")
        # index_state "building": set switching is disabled until it is ready.
        def building(route):
            r = route.fetch()
            route.fulfill(response=r, json={**r.json(), "index_state": "building"})
        page.route("**/api/status", building)
        page.evaluate("async () => { const { app } = await import('./js/state.js'); await app.refreshStatus(); }")
        page.wait_for_function("document.querySelector('#st-set').disabled", timeout=5000)
        check("building" in page.inner_text("#st-facts"), "settings: building -> set select disabled")
        shot(page, "settings-building", "laptop")
        goto_view(page, "sets")
        page.wait_for_function("[...document.querySelectorAll('#sets-list [data-act]')].every(b => b.disabled)", timeout=5000)
        check(page.locator("#sets-list [data-act]").count() > 0, "sets: building -> every Activate disabled")
        shot(page, "sets-building", "laptop")
        page.unroute("**/api/status")
        goto_view(page, "settings")
        page.wait_for_function("!document.querySelector('#st-set').disabled", timeout=8000)
        check(True, "settings: ready again -> set select enabled")
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
        send_from_library(page, base, "device library")
        shot(page, "device-library", "laptop")
        goto_view(page, "settings")
        page.wait_for_selector("#st-facts .stat", timeout=20000)
        check(page.input_value("#st-hostname") != "", "device settings: config GET rendered")
        st = get_json(base + "/api/status")
        facts = page.inner_text("#st-facts")
        check(st.get("index_state") in facts and (st.get("www_version") or "–") in facts,
              f"device settings: index_state {st.get('index_state')}, www_version {st.get('www_version')} shown")
        shot(page, "device-settings", "laptop")
        # The one write: save and delete a set named smoke (never activated).
        n = build_smoke_set(page)
        rows = root_manifest_rows(page)
        dflt = page.eval_on_selector_all("#sets-list [data-path] .tag:not(.act)", "els => els.filter(e => e.textContent === 'default').map(e => e.closest('[data-path]').dataset.path)")
        check("/filelist.txt" in rows and dflt == ["/" + st["manifest"].rsplit("/", 1)[1]] if get_json(base + "/api/config")["set"] == "" else len(dflt) == 1,
              f"device sets: root manifests {rows}, default {dflt}")
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
