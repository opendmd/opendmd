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

  Now       status renders, the preview lights pixels; the preview is in
            step with the mock's clock (drift < 60 ms after two polls, the
            mock's position running through the file's real frame delays),
            re-syncs to the new file after a skip, pauses when the mock is
            paused and holds its frame; skip, pause from the UI; "Send
            to panel" of a card path (POST /api/play) shows in status, and a
            missing path / a non-GIF give the 404 / 415 toasts
  Library   open Consoles, search "mario", platform chip SNES, preview one,
            send it to the panel, select three
  Sets      the playlists are listed by name (All files — the default —,
            Halloween, Christmas; never the obsolete filelist.odm.*);
            activate Christmas (set "filelist.xmas") and All files back as
            ""; build from platforms MD,SNES, save as `smoke` (the file must
            hold exactly the matching filelist.txt lines, in order), activate
            it, delete it (the UI switches back to the default first)
  Cache     the three selected GIFs as a playlist `cachetest`, activated;
            their prepared copies forgotten (POST /api/cache/forget), so the
            Settings card reads "0 of 3"; "Prepare remaining files" makes all
            three, byte-identical to `opendmd-pack convert --layout
            bucketed`, and the card reads "3 of 3"
  Add GIFs  two GIFs uploaded into Consoles from Library: each GIF and its
            prepared copy land (the copy byte-identical to opendmd-pack), and
            the confirm adds both to `cachetest` ("5 of 5"); deleting one of
            the GIFs takes its copy with it
  Settings  www_version and index_state shown; PUT brightness, read it
            back; WiFi scan; the playlist choices are labelled; with
            /api/status rewritten to index_state "building" (Playwright
            route), set switching is disabled in Settings and Sets, and comes
            back when it reads "ready"

Every view's visible text (and every title tooltip) is checked for "odm" and
"bucket": prepared copies are an implementation detail.

A few requests fail on purpose (the 404/415 plays, and a preview's try for a
prepared copy that is not there); they are listed in `expected` and are not
counted as console errors.

Real-device pass (--device): the local build served through
`mock-device.py --proxy`, so nothing is uploaded to the card's www/. Now
(and a minute of the preview's drift against the panel: median, max, RTT —
DRIFT_S=60 by default),
Library (+ a preview, sent to the panel with POST /api/play, which must show
up in /api/status), a preview of a GIF that has a prepared copy (fetched from
the copy, never the GIF), the cache count (/api/cache, waited until ready),
the playlists listed by name, Settings (read only) and one write: save a set
named `smoke` and delete it. No preparing, no filelist or config changes.

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
import urllib.parse
import urllib.request
from pathlib import Path

from playwright.sync_api import sync_playwright, Page

REPO = Path(__file__).resolve().parents[1]
CARD = Path(os.environ.get("DMD_CARD_ROOT", "~/scratch/dmdos-card"))
OUT = Path(os.environ.get("WEB_SMOKE_OUT", REPO / "target" / "web-smoke"))
SIZES = {"laptop": (1280, 800), "phone": (390, 844)}
VIEWS = ["now", "library", "sets", "settings"]

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


def sync_state(page: Page) -> dict:
    """The Now preview's last sync (js/now.js puts it on #now-drift)."""
    return page.evaluate("() => ({...document.querySelector('#now-drift').dataset})")


def wait_sync(page: Page, k: int, not_path: str | None = None, timeout_s: float = 15) -> dict:
    """Until the preview has synced `k` times to one file (not `not_path`)."""
    deadline = time.time() + timeout_s
    st = {}
    while time.time() < deadline:
        st = sync_state(page)
        if int(st.get("k", 0)) >= k and st.get("path") != not_path:
            break
        page.wait_for_timeout(100)
    return st


def drift_stats(page: Page, seconds: float) -> tuple[list[float], list[float]]:
    """Every sync's drift and RTT for `seconds`, leaving out each file's
    first sync (the jump from free-running into step). Prints the range of
    the measured position rates."""
    drifts, rtts, rates, seen = [], [], [], None
    deadline = time.time() + seconds
    while time.time() < deadline:
        st = sync_state(page)
        key = (st.get("path"), st.get("k"))
        if key != seen and int(st.get("k", 0)) >= 2:
            drifts.append(float(st["drift"]))
            rtts.append(float(st["rtt"]))
            rates.append(float(st["rate"]))
        seen = key
        page.wait_for_timeout(50)
    if rates:
        print(f"  position rate (panel ms per ms): {min(rates):.4f}..{max(rates):.4f}")
    return drifts, rtts


def summarise(what: str, drifts: list[float], rtts: list[float]) -> str:
    if not drifts:
        return f"{what}: no syncs"
    a = sorted(abs(d) for d in drifts)
    r = sorted(rtts)
    return (f"{what}: {len(a)} syncs, |drift| median {a[len(a) // 2]:.1f} ms, max {a[-1]:.1f} ms, "
            f"p90 {a[int(len(a) * 0.9)]:.1f} ms; rtt median {r[len(r) // 2]:.1f} ms, min {r[0]:.1f} ms")


def canvas_sum(page: Page, sel: str) -> int:
    return page.evaluate("""(sel) => {
        const c = document.querySelector(sel);
        const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
        let n = 0;
        for (let i = 0; i < d.length; i += 4) n = (n * 31 + d[i] + d[i+1] * 3 + d[i+2] * 7) >>> 0;
        return n;
    }""", sel)


def shot(page: Page, name: str, size: str) -> None:
    path = OUT / f"{name}-{size}.png"
    page.screenshot(path=str(path))
    print(f"  shot {path}")


# URL substrings of requests that fail on purpose (a 404/415 play). A
# preview asks for the prepared copy first, and a 404 there is normal.
expected: list[str] = ["/odm/"]


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


def row_label(page: Page, path: str) -> str:
    return page.eval_on_selector(f"#sets-list [data-path='{path}'] .grow", "e => e.firstChild.textContent")


# Visible copy that would leak the implementation: prepared copies are a
# cache the user never has to know the format or layout of.
LEAK = re.compile(r"odm|bucket", re.I)


# Names of playlists the user made are their content, not UI copy (the rig's
# card has bench sets called "odm" and "odmb"); filled in by device_pass.
user_names: set[str] = set()


def no_leaks(page: Page, where: str) -> None:
    text = page.evaluate("""() => [document.body.innerText,
        ...[...document.querySelectorAll('[title],[placeholder]')].map(e => (e.title || '') + ' ' + (e.placeholder || '')),
        ...[...document.querySelectorAll('option')].map(o => o.textContent)].join('\\n')""")
    for n in sorted(user_names, key=len, reverse=True):
        text = re.sub(rf"(?<![\w.]){re.escape(n)}(?![\w.])", "", text)
    hits = sorted({m.group(0) for m in LEAK.finditer(text)})
    lines = [l for l in text.split("\n") if LEAK.search(l)][:3]
    check(not hits, f"{where}: no 'odm'/'bucket' in visible text {lines if hits else ''}")


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


def post_json(url: str, obj) -> dict:
    req = urllib.request.Request(url, data=json.dumps(obj).encode(), method="POST",
                                 headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.load(r)


def cache_ready(base: str, timeout_s: float = 60) -> dict:
    """GET /api/cache until it stops counting."""
    deadline = time.time() + timeout_s
    r = get_json(base + "/api/cache")
    while r["state"] != "ready" and time.time() < deadline:
        time.sleep(0.5)
        r = get_json(base + "/api/cache")
    return r


def wait_cache_line(page: Page, text: str, timeout_ms: int = 60000) -> str:
    page.wait_for_function(f"document.querySelector('#pc-count').textContent.startsWith({json.dumps(text)})", timeout=timeout_ms)
    return page.inner_text("#pc-count")


def mock_pass(browser) -> None:
    overlay = OUT / "overlay"
    shutil.rmtree(overlay, ignore_errors=True)
    overlay.mkdir(parents=True)
    proc, base = start_mock(["--root", str(CARD), "--overlay", str(overlay), "--dwell", "4"])
    errors: list[str] = []
    try:
        print(f"\n== mock device at {base} (overlay {overlay})")
        page = new_page(browser, "laptop", errors)
        page.goto(base + "/")
        # Now
        page.wait_for_function("document.querySelector('#now-path') && document.querySelector('#now-path').textContent.startsWith('/')", timeout=20000)
        check(page.locator("#now-stats .stat").count() >= 8, "now: status grid renders")
        check(get_json(base + "/api/status")["manifest"] == "/sdcard/filelist.txt", "now: the default playlist is filelist.txt")
        wait_lit(page, "#now-panel", "now")
        # In step with the mock's clock: a first sync, then two polls.
        st = wait_sync(page, 3)
        check(abs(float(st.get("drift", 1e9))) < 60, f"now: preview in step after two polls (drift {st.get('drift')} ms, rtt {st.get('rtt')} ms, {st.get('path')})")
        check(page.inner_text("#now-cap") == "preview · in step with the panel", "now: caption says in step")
        # A skip on the device (not through the UI): the preview loads the
        # new file and is in step with it.
        post_json(base + "/api/skip", {})
        st = wait_sync(page, 3, not_path=st.get("path"))
        now_path = get_json(base + "/api/status")["now_playing"]["path"]
        check(st.get("path") == now_path and abs(float(st.get("drift", 1e9))) < 60,
              f"now: a skip re-syncs to the new file (drift {st.get('drift')} ms, {st.get('path')})")
        # Paused on the device: the preview pauses and holds its frame.
        post_json(base + "/api/pause", {})
        page.wait_for_function("document.querySelector('#now-drift').dataset.paused === 'true'", timeout=5000)
        page.wait_for_timeout(300)
        a = canvas_sum(page, "#now-panel")
        page.wait_for_timeout(1200)
        check(a == canvas_sum(page, "#now-panel"), "now: pausing the mock pauses the preview (frame held 1.2 s)")
        post_json(base + "/api/pause", {})
        page.wait_for_function("document.querySelector('#now-drift').dataset.paused === 'false'", timeout=5000)
        k = int(sync_state(page)["k"])
        st = wait_sync(page, k + 2)
        check(abs(float(st.get("drift", 1e9))) < 60, f"now: resumed, still in step (drift {st.get('drift')} ms)")
        drifts, rtts = drift_stats(page, 8)
        print("  " + summarise("mock drift over 8 s", drifts, rtts))
        check(bool(drifts) and max(abs(d) for d in drifts) < 60, "now: every sync on the mock under 60 ms")
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
        check("not a GIF" in t and not LEAK.search(t), f"now: 415 play -> toast '{t}'")
        page.evaluate("document.querySelectorAll('#toasts .toast').forEach(t => t.remove())")
        page.fill("#now-play-path", "")
        no_leaks(page, "now")
        shot(page, "now", "laptop")
        expected.remove("/api/play")
        # Library, and three Mario GIFs for the cache test
        open_library(page)
        send_from_library(page, base, "library")
        page.eval_on_selector_all("#lib-list .vrow:not([hidden]) input[type=checkbox]",
                                  "els => els.slice(0, 3).forEach(e => e.click())")
        check(page.inner_text("#lib-selbar b") == "3", "library: three selected")
        selected = page.evaluate("async () => { const { app } = await import('./js/state.js'); return [...app.selection]; }")
        no_leaks(page, "library")
        shot(page, "library", "laptop")
        # Sets: the playlists by name, the default, activation by value
        goto_view(page, "sets")
        page.wait_for_selector("#sets-list [data-path='/filelist.xmas']", timeout=20000)
        rows = root_manifest_rows(page)
        check({"/filelist.txt", "/filelist.hall", "/filelist.xmas"} <= set(rows) and not any("odm" in r for r in rows),
              f"sets: root playlists listed, the obsolete ones not ({rows})")
        labels = {p: row_label(page, p) for p in ["/filelist.txt", "/filelist.hall", "/filelist.xmas"]}
        check(labels == {"/filelist.txt": "All files", "/filelist.hall": "Halloween", "/filelist.xmas": "Christmas"},
              f"sets: friendly labels {labels}")
        dflt = page.eval_on_selector_all("#sets-list [data-path] .tag:not(.act)", "els => els.filter(e => e.textContent === 'default').map(e => e.closest('[data-path]').dataset.path)")
        check(dflt == ["/filelist.txt"], f"sets: default marked on {dflt}")
        check("All files" in page.inner_text("#set-active"), f"sets: '{page.inner_text('#set-active')}'")
        page.click("#sets-list [data-path='/filelist.xmas'] >> text=Activate")
        page.wait_for_function("document.querySelector('#set-active').textContent.includes('Christmas')", timeout=60000)
        st, cf = get_json(base + "/api/status"), get_json(base + "/api/config")
        check(cf["set"] == "filelist.xmas" and st["manifest"] == "/sdcard/filelist.xmas",
              f"sets: Christmas activated as set \"filelist.xmas\" ({st['entries']} entries)")
        no_leaks(page, "sets")
        shot(page, "sets-root", "laptop")
        page.click("#sets-list [data-path='/filelist.txt'] >> text=Activate")
        page.wait_for_function("document.querySelector('#set-active').textContent.includes('All files')", timeout=60000)
        st, cf = get_json(base + "/api/status"), get_json(base + "/api/config")
        check(cf["set"] == "" and st["manifest"] == "/sdcard/filelist.txt", "sets: All files activated as set \"\"")
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
        # Cache: the three selected GIFs as a playlist, their copies forgotten.
        goto_view(page, "library")
        page.click("#lib-to-set")
        page.wait_for_selector("#view-sets.on")
        page.evaluate("async () => { const { app } = await import('./js/state.js'); const { emit } = await import('./js/util.js'); app.draft.paths = [...app.selection]; app.draft.name = ''; emit('draft'); }")
        page.fill("#set-name", "cachetest")
        page.click("#set-save-activate")
        page.wait_for_function("document.querySelector('#set-active').textContent.includes('cachetest')", timeout=60000)
        check(get_json(base + "/api/status")["entries"] == 3, "cache: playlist cachetest (the three selected) playing")
        forgot = [post_json(base + "/api/cache/forget", {"path": p})["removed"] for p in selected]
        check(all(forgot), f"cache: POST /api/cache/forget removed the three copies ({forgot})")
        goto_view(page, "settings")
        line = wait_cache_line(page, "0 of 3")
        check(True, f"cache: settings card reads '{line}'")
        no_leaks(page, "settings (cache)")
        page.click("#pc-go")
        page.wait_for_function("document.querySelector('#pc-progress').textContent.startsWith('Done')", timeout=120000)
        line = wait_cache_line(page, "3 of 3")
        check(True, f"cache: prepared -> '{line}' ({page.inner_text('#pc-progress')})")
        made = {bucketed(p): CARD / p.lstrip("/") for p in selected}
        compare_with_pack(overlay, made)
        no_leaks(page, "settings (prepared)")
        shot(page, "settings-cache", "laptop")
        # Add GIFs: two new files into Consoles, prepared, added to cachetest.
        src_dir = OUT / "add-src"
        shutil.rmtree(src_dir, ignore_errors=True)
        src_dir.mkdir(parents=True)
        adds = []
        for i, p in enumerate(selected[:2]):
            dst = src_dir / f"SNES_Smoke_Added_{i + 1:02d}.gif"
            shutil.copy(CARD / p.lstrip("/"), dst)
            adds.append(dst)
        goto_view(page, "library")
        page.select_option("#lib-cat", "Consoles")
        page.wait_for_function("document.querySelector('#lib-count').textContent.includes(' of ')", timeout=60000)
        page.set_input_files("#lib-add-files", [str(a) for a in adds])
        page.wait_for_function("document.querySelector('#lib-add-progress').textContent.startsWith('2 of 2 added')", timeout=60000)
        page.wait_for_function("[...document.querySelectorAll('#toasts .toast.ok')].some(t => t.textContent.includes('cachetest'))", timeout=60000)
        gif_paths = [f"/gifs/Consoles/{a.name}" for a in adds]
        landed = [(overlay / p.lstrip("/")).exists() for p in gif_paths]
        check(all(landed), f"add: both GIFs uploaded ({landed})")
        compare_with_pack(overlay, {bucketed(p): a for p, a in zip(gif_paths, adds)})
        plist = (overlay / "sets" / "cachetest.txt").read_text().split("\n")
        check(plist[3:5] == gif_paths and get_json(base + "/api/status")["entries"] == 5,
              f"add: both appended to cachetest ({len([l for l in plist if l])} lines)")
        r = cache_ready(base)
        check((r["prepared"], r["total"]) == (5, 5), f"add: /api/cache {r['prepared']} of {r['total']}")
        no_leaks(page, "library (add)")
        shot(page, "library-add", "laptop")
        req = urllib.request.Request(base + "/api/files" + gif_paths[1].replace(" ", "%20"), method="DELETE")
        urllib.request.urlopen(req, timeout=30).read()
        gone = not (overlay / bucketed(gif_paths[1]).lstrip("/")).exists()
        r = cache_ready(base)
        check(gone and r["prepared"] == 4, f"add: deleting a GIF took its copy with it ({r['prepared']} of {r['total']})")
        # Back to All files; the test playlist goes.
        goto_view(page, "sets")
        page.click("#sets-list [data-path='/sets/cachetest.txt'] >> text=Delete")
        page.wait_for_selector("#sets-list [data-path='/sets/cachetest.txt']", state="detached", timeout=60000)
        check(get_json(base + "/api/config")["set"] == "", "cache: cachetest deleted, All files playing")
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
        opts = page.eval_on_selector_all("#st-set option", "els => els.map(e => [e.value, e.textContent])")
        check(opts[0] == ["", "All files (default)"] and ["filelist.xmas", "Christmas"] in opts
              and not any("odm" in v for v, _ in opts), f"settings: playlist choices {opts[:4]}")
        no_leaks(page, "settings")
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
        check(page.locator("#nav a[data-view]").count() == 4 and page.locator("#view-convert").count() == 0,
              "nav: four views, no Convert")
        phone_pass(browser, base, errors)
    finally:
        proc.terminate()
    report_errors("mock", errors)


def compare_with_pack(overlay: Path, made: dict[str, Path]) -> None:
    """Run opendmd-pack on the same GIFs and compare the uploaded copies byte
    for byte. `made` maps a copy's card path to the GIF it was made from."""
    mini = OUT / "pack-ref"
    shutil.rmtree(mini, ignore_errors=True)
    for copy, gif in made.items():
        cat = copy.split("/")[2]
        (mini / "gifs" / cat).mkdir(parents=True, exist_ok=True)
        shutil.copy(gif, mini / "gifs" / cat / gif.name)
    subprocess.run(["cargo", "run", "--release", "--quiet", "--", "convert", str(mini), "--layout", "bucketed", "--verify"],
                   cwd=REPO / "opendmd-pack", check=True, env={**os.environ, "RUSTUP_TOOLCHAIN": "stable"},
                   stdout=subprocess.DEVNULL)
    for copy in made:
        got = overlay / copy.lstrip("/")
        ref = mini / copy.lstrip("/")
        same = got.exists() and ref.exists() and filecmp.cmp(ref, got, shallow=False)
        size = got.stat().st_size if got.exists() else 0
        check(same, f"prepared copy {copy} byte-identical to opendmd-pack ({size} B)")


def phone_pass(browser, base: str, errors: list[str]) -> None:
    page = new_page(browser, "phone", errors)
    page.goto(base + "/")
    page.wait_for_function("document.querySelector('#now-path') && document.querySelector('#now-path').textContent.startsWith('/')", timeout=20000)
    wait_lit(page, "#now-panel", "phone now")
    shot(page, "now", "phone")
    for view in ["library", "sets", "settings"]:
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
        try:
            user_names.update(e["name"][:-4] for e in get_json(base + "/api/files?dir=/sets")["entries"]
                              if e["name"].endswith(".txt"))
        except Exception:  # noqa: BLE001 - no /sets on the card
            pass
        page = new_page(browser, "laptop", errors)
        page.goto(base + "/")
        page.wait_for_function("document.querySelector('#now-path') && document.querySelector('#now-path').textContent.startsWith('/')", timeout=30000)
        check(page.locator("#now-stats .stat").count() >= 8, "device now: status grid renders")
        wait_lit(page, "#now-panel", "device now", 40)
        st0 = get_json(base + "/api/status")
        check(all(k in st0["now_playing"] for k in ("elapsed_ms", "frame")) and "t_ms" in st0,
              "device now: status has now_playing.elapsed_ms/frame and t_ms")
        secs = float(os.environ.get("DRIFT_S", "60"))
        drifts, rtts = drift_stats(page, secs)
        st1 = get_json(base + "/api/status")
        print("  " + summarise(f"device drift over {secs:.0f} s", drifts, rtts))
        print(f"  device over that minute: misses {st0['deadline_misses']} -> {st1['deadline_misses']}, "
              f"worst {st1['worst_lateness_ms']} ms, files {st0['gifs_started']} -> {st1['gifs_started']}")
        check(len(drifts) > secs, f"device now: the preview kept syncing ({len(drifts)} syncs)")
        no_leaks(page, "device now")
        shot(page, "device-now", "laptop")
        open_library(page)
        send_from_library(page, base, "device library")
        shot(page, "device-library", "laptop")
        # A preview of a GIF that has a prepared copy: fetched from the copy.
        t0 = time.time()
        copies = get_json(base + "/api/cache/list")["paths"]
        list_s = time.time() - t0
        check(len(copies) > 0, f"device cache: /api/cache/list {len(copies)} copies in {list_s:.1f} s")
        pick = next((c for c in copies if c.count("/") == 4), None)
        if pick:
            _, _, cat, _, name = pick.split("/")
            gif = f"/gifs/{cat}/{name[:-4]}.gif"
            reqs: list[tuple[str, int]] = []
            page.on("response", lambda r: reqs.append((urllib.parse.unquote(r.url), r.status)))
            goto_view(page, "library")
            page.select_option("#lib-cat", cat)
            page.wait_for_function("document.querySelector('#lib-count').textContent.includes(' of ')", timeout=60000)
            for chip in page.query_selector_all("#lib-chips .chip.on"):
                chip.click()
            page.fill("#lib-search", name[:-4])
            page.wait_for_timeout(500)
            page.click(f"#lib-list .vrow:not([hidden]) >> nth=0")
            page.wait_for_function(f"document.querySelector('#lib-cap').textContent.endsWith({json.dumps(name[:-4] + '.gif')})", timeout=40000)
            wait_lit(page, "#lib-panel", f"device library {gif}", 40)
            fetched = [(u.split(base, 1)[-1], s) for u, s in reqs if "/odm/" in u or "/gifs/" in u]
            check((pick, 200) in fetched and not any(u == gif for u, _ in fetched),
                  f"device library: {gif} previewed from its prepared copy ({fetched})")
            check(page.inner_text("#lib-cap").endswith(name[:-4] + ".gif"), f"device library: caption names the GIF ({page.inner_text('#lib-cap')})")
            no_leaks(page, "device library")
        # The cache count.
        t0 = time.time()
        r = cache_ready(base, 180)
        check(r["state"] == "ready" and r["prepared"] is not None,
              f"device cache: /api/cache {r['prepared']} of {r['total']} prepared (count {r['count_ms']} ms, ready after {time.time() - t0:.1f} s)")
        goto_view(page, "settings")
        page.wait_for_selector("#st-facts .stat", timeout=20000)
        check(page.input_value("#st-hostname") != "", "device settings: config GET rendered")
        st = get_json(base + "/api/status")
        facts = page.inner_text("#st-facts")
        check(st.get("index_state") in facts and (st.get("www_version") or "–") in facts,
              f"device settings: index_state {st.get('index_state')}, www_version {st.get('www_version')} shown")
        line = wait_cache_line(page, f"{r['prepared']:,} of {r['total']:,}")
        check(True, f"device settings: cache card reads '{line}'")
        no_leaks(page, "device settings")
        shot(page, "device-settings", "laptop")
        # The one write: save and delete a set named smoke (never activated).
        n = build_smoke_set(page)
        rows = root_manifest_rows(page)
        labels = {p: row_label(page, p) for p in rows}
        check("/filelist.txt" in rows and labels.get("/filelist.txt") == "All files"
              and not any("odm" in p for p in rows), f"device sets: playlists {labels}")
        no_leaks(page, "device sets")
        with urllib.request.urlopen(base + "/sets/smoke.txt", timeout=30) as resp:
            body = resp.read().decode()
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
