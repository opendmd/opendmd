# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright>=1.45", "pillow"]
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
  Damaged   a playlist of two of them (copies forgotten) and two damaged
            GIFs written into the overlay (cut short, broken header); the
            first GET of one good GIF aborted (Playwright route) is retried:
            one summary line "2 files prepared · 2 damaged files skipped
            (they can't be played either).", both listed in the disclosure
            and in /api/cache/failed; a second run never fetches them
  Add GIFs  two GIFs uploaded into Consoles from Library: each GIF and its
            prepared copy land (the copy byte-identical to opendmd-pack), and
            the confirm adds both to `cachetest` ("5 of 5"); deleting one of
            the GIFs takes its copy with it
  Studio    "Make art" from Library: a CC0 PNG (a crop of
            experiments/art/process/out/a_source_windows.png, a 3x copy of
            a Kenney scene) imported; the 3x found, Pixels, Pan, the auto
            band; saved as SNES_Studio_Test_01.gif: the GIF and its prepared
            copy land where they belong (the copy byte-identical to
            opendmd-pack's), the GIF's frames and delays equal the Rust
            pipeline's (`cargo run --example render` in opendmd-art-web),
            parsePath of the name gives SNES / "Studio Test" / 1, and it is
            appended to `cachetest`; "Find art for a game" against a stubbed
            name index and stubbed libretro pictures (one of them a link to
            another), no network
  Auto      preparing by the device (auto_prepare): /api/cache `auto` and
            /api/status `cache` keys; the card's sentence while playing
            ("…when the panel is idle · 0 of 3"), with the mock paused (its
            simulated builder at full pace, "about N s of idle time left")
            and in night mode ("about N nights left"), "All files are
            prepared" after preparing; the option ticked -> "always",
            unticked -> "idle"; a bad value is a 400; a shot of the card
  Settings  www_version and index_state shown; PUT brightness, read it
            back; WiFi scan; the playlist choices are labelled; with
            /api/status rewritten to index_state "building" (Playwright
            route), set switching is disabled in Settings and Sets, and comes
            back when it reads "ready"

Every view's visible text (and every title tooltip) is checked for "odm" and
"bucket": prepared copies are an implementation detail.

A few requests fail on purpose (the 404/415 plays, a preview's try for a
prepared copy that is not there, and the aborted GIF download); they are
listed in `expected` and are not counted as console errors.

Real-device pass (--device): the local build served through
`mock-device.py --proxy`, so nothing is uploaded to the card's www/. Now
(and a minute of the preview's drift against the panel: median, max, RTT —
DRIFT_S=60 by default),
Library (+ a preview, sent to the panel with POST /api/play, which must show
up in /api/status), a preview of a GIF that has a prepared copy (fetched from
the copy, never the GIF), the cache count (/api/cache, waited until ready),
the playlists listed by name, Settings (read only; the auto-prepare fields
and sentence, the setting left alone), the Studio (opens, "Find art for a
game" finds Super Metroid in the real SNES index and loads its pictures
from GitHub; nothing saved) and one write: save a set
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
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime, timedelta
from pathlib import Path
from zoneinfo import ZoneInfo

from playwright.sync_api import sync_playwright, Page
from playwright.sync_api import TimeoutError as PlaywrightTimeoutError

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


def start_replay(token: str = "4321", speed: float = 1.0) -> tuple[subprocess.Popen, str, int]:
    """A fake RePlayOS (tools/mock-replay.py) on a free port, for the Follow
    RePlayOS checks. Returns (process, base url, port)."""
    port = free_port()
    proc = subprocess.Popen(
        ["uv", "run", str(REPO / "tools" / "mock-replay.py"), "--port", str(port),
         "--token", token, "--speed", str(speed), "--quiet"],
        stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT)
    for _ in range(100):
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                return proc, f"http://127.0.0.1:{port}", port
        except OSError:
            time.sleep(0.1)
    proc.kill()
    raise RuntimeError("mock-replay.py did not come up")


def replay_post(replay_base: str, path: str) -> None:
    """A no-body POST to one of mock-replay's control endpoints."""
    req = urllib.request.Request(replay_base + path, data=b"", method="POST")
    with urllib.request.urlopen(req, timeout=10) as r:
        r.read()


def replay_goto(replay_base: str, step: int) -> None:
    replay_post(replay_base, f"/mock/goto?step={step}")


def replay_set_token(replay_base: str, value: str) -> None:
    replay_post(replay_base, "/mock/token?value=" + urllib.parse.quote(value, safe=""))


def wait_context(base: str, ok, timeout_s: float = 20) -> dict:
    """Poll /api/status's `context` (Follow RePlayOS) until `ok(context)`."""
    deadline = time.time() + timeout_s
    c = {}
    while True:
        c = get_json(base + "/api/status")["context"]
        if ok(c) or time.time() > deadline:
            return c
        time.sleep(0.15)


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


# The Now preview, hooked at Preview.load: the canvas just before the new
# file goes in (the old picture), and the new file's path and bytes.
HOOK_LOAD = """async () => {
    const m = await import('./pkg/opendmd_web.js');
    const P = m.Preview.prototype;
    P.__orig ??= P.load;
    window.__tr = null;
    P.load = function (path, bytes) {
        const c = document.querySelector('#now-panel');
        if (c.offsetParent !== null && !window.__tr)
            window.__tr = { path, bytes: bytes.slice(), at: performance.now(),
                            before: c.getContext('2d').getImageData(0, 0, c.width, c.height).data };
        return P.__orig.call(this, path, bytes);
    };
}"""

# The Now preview now, against the picture before the load and against the
# new file's first frame rendered by a second Preview with no transition.
# Pixels count as different when their RGB differs by more than 24 in sum.
MID_TRANSITION = """async () => {
    const c = document.querySelector('#now-panel');
    const mid = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
    const dt = performance.now() - window.__tr.at;
    const m = await import('./pkg/opendmd_web.js');
    m.Preview.prototype.load = m.Preview.prototype.__orig;
    const { look } = await import('./js/preview.js');
    const o = document.createElement('canvas');
    o.width = c.width;
    const p = new m.Preview(o, look.gamma, look.gain, look.bloom, look.flat);
    p.setTransition('none', 400);
    p.load(window.__tr.path, window.__tr.bytes);
    p.frame();
    const first = o.getContext('2d').getImageData(0, 0, o.width, o.height).data;
    p.free();
    const diff = (a, b) => { let n = 0; for (let i = 0; i < a.length; i += 4)
        if (Math.abs(a[i] - b[i]) + Math.abs(a[i+1] - b[i+1]) + Math.abs(a[i+2] - b[i+2]) > 24) n++; return n; };
    return { dt, path: window.__tr.path, vs_before: diff(mid, window.__tr.before), vs_first: diff(mid, first),
             before_first: diff(window.__tr.before, first) };
}"""


def wait_config(base: str, ok, timeout_s: float = 10) -> dict:
    deadline = time.time() + timeout_s
    while True:
        cfg = get_json(base + "/api/config")
        if ok(cfg) or time.time() > deadline:
            return cfg
        time.sleep(0.2)


def mid_transition(page: Page, base: str) -> dict:
    """Skip on the device; ~200 ms after the Now preview loads the new file
    (the transition's start), compare the preview with the old picture and
    with the new file's first frame. Tries up to three skips for a pair of
    files that differ enough to tell."""
    r = {}
    for _ in range(3):
        wait_sync(page, 2)
        page.evaluate(HOOK_LOAD)
        post_json(base + "/api/skip", {})
        page.wait_for_function("window.__tr !== null", timeout=15000)
        page.wait_for_function("performance.now() - window.__tr.at >= 200", timeout=5000)
        r = page.evaluate(MID_TRANSITION)
        if r["before_first"] > 2000:
            break
    return r


def transition_checks(page: Page, base: str) -> None:
    """Settings -> Transition: save Dissolve and Wipe (Normal), and check the
    Now preview is mid-transition 200 ms into a new file each time; the card's
    Preview button plays one on the page; bad values are refused."""
    cfg = get_json(base + "/api/config")
    check(cfg.get("transition") == "dissolve" and cfg.get("transition_ms") == 400,
          f"settings: transition in /api/config ({cfg.get('transition')}, {cfg.get('transition_ms')})")
    for bad in ({"transition": "sparkle"}, {"transition_ms": 5}, {"transition_ms": 9000}):
        try:
            put_json(base + "/api/config", bad)
            check(False, f"settings: {bad} refused")
        except urllib.error.HTTPError as e:
            check(e.code == 400, f"settings: {bad} refused ({e.code})")
    for kind, label in (("dissolve", "Dissolve"), ("wipe_left", "Wipe")):
        goto_view(page, "settings")
        page.select_option("#st-trans", kind)
        page.select_option("#st-trans-ms", "400")
        # Saving the same value is "nothing changed"; flip the speed first.
        if get_json(base + "/api/config")["transition"] == kind:
            put_json(base + "/api/config", {"transition_ms": 250})
            page.evaluate("async () => { const { app } = await import('./js/state.js'); await app.refreshConfig(); }")
        page.click("#st-save-trans")
        cfg = wait_config(base, lambda c: c["transition"] == kind and c["transition_ms"] == 400)
        check(cfg["transition"] == kind and cfg["transition_ms"] == 400, f"settings: {label} / Normal saved ({cfg['transition']}:{cfg['transition_ms']})")
        goto_view(page, "now")
        r = mid_transition(page, base)
        check(r["vs_before"] > 0.1 * r["before_first"] and r["vs_first"] > 0.1 * r["before_first"],
              f"now: {label} 400 ms, {r['dt']:.0f} ms after the new file loaded the preview differs from the old "
              f"picture ({r['vs_before']} px) and from the new file's first frame ({r['vs_first']} px); "
              f"those two differ by {r['before_first']} px ({r['path']})")
    # The card's Preview: the panel's file, restarted with a transition, on the page only.
    goto_view(page, "settings")
    page.select_option("#st-trans", "blinds")
    before = get_json(base + "/api/status")["now_playing"]
    page.click("#st-trans-preview")
    wait_lit(page, "#st-trans-panel", "settings transition preview")
    page.wait_for_timeout(700)
    page.click("#st-trans-preview")
    page.wait_for_timeout(150)
    card = OUT / "settings-transition-laptop.png"
    page.locator("#st-trans-card").screenshot(path=str(card))
    print(f"  shot {card}")
    after = get_json(base + "/api/status")
    check(get_json(base + "/api/config")["transition"] == "wipe_left" and after["now_playing"]["ordinal"] == before["ordinal"],
          "settings: Preview plays on the page only (config and the panel untouched)")
    page.select_option("#st-trans", "dissolve")
    page.click("#st-save-trans")
    check(wait_config(base, lambda c: c["transition"] == "dissolve")["transition"] == "dissolve", "settings: back to Dissolve")
    no_leaks(page, "settings (transition)")
    goto_view(page, "now")


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
# cache the user never has to know the format or layout of, and per-GIF
# overrides are "favourite"/"hide"/"play longer"/"loop N times" -- never
# their weight, file format ('.meta') or raw field syntax ('w=').
LEAK = re.compile(r"odm|bucket|weight|\.meta|w=|never", re.I)

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
    check(not hits, f"{where}: no 'odm'/'bucket'/'weight'/'.meta'/'w='/'never' in visible text {lines if hits else ''}")


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


def put_json(url: str, obj) -> dict:
    req = urllib.request.Request(url, data=json.dumps(obj).encode(), method="PUT",
                                 headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.load(r)


def put_text(url: str, text: str) -> dict:
    req = urllib.request.Request(url, data=text.encode(), method="PUT")
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.load(r)


# --- Per-GIF overrides: opendmd_core::overrides, mirrored for expectations ---

def fmt_override_decimal(v: float) -> str:
    """overrides::push_decimal: rounded to 3 places, trailing zeros dropped."""
    milli = int(max(float(v), 0.0) * 1000.0 + 0.5)
    whole, frac = divmod(milli, 1000)
    if frac == 0:
        return str(whole)
    f = f"{frac:03d}".rstrip("0")
    return f"{whole}.{f}"


def render_meta_line(path: str, ov: dict) -> str:
    """The canonical `.meta` line for one GIF: fields in order never, w,
    dwell/dwell_x, loops (overrides::render_fields)."""
    parts = [path]
    if ov.get("never"):
        parts.append("never")
    if "w" in ov:
        parts.append(f"w={fmt_override_decimal(ov['w'])}")
    if "dwell" in ov:
        parts.append(f"dwell={ov['dwell']}")
    elif "dwell_x" in ov:
        parts.append(f"dwell={fmt_override_decimal(ov['dwell_x'])}x")
    if "loops" in ov:
        parts.append(f"loops={ov['loops']}")
    return "\t".join(parts)


def wait_overrides(base: str, set_value: str, n: int, timeout_s: float = 10) -> dict:
    """Poll GET /api/overrides?set= until it holds at least `n` entries (the
    Overrides class saves one write at a time, in the background)."""
    deadline = time.time() + timeout_s
    r = {"set": set_value, "playlist": "", "overrides": []}
    while time.time() < deadline:
        r = get_json(base + "/api/overrides?set=" + urllib.parse.quote(set_value, safe=""))
        if len(r["overrides"]) >= n:
            return r
        time.sleep(0.1)
    return r


def open_row_menu(page: Page, list_sel: str, idx: int, via: str = "more") -> None:
    """Open the overrides menu for the row at `idx` among the currently
    rendered (unhidden, pooled) rows of a VList -- its "…" button, or a
    right-click on the row itself."""
    row = f"{list_sel} .vrow:not([hidden]) >> nth={idx}"
    if via == "more":
        page.click(f"{row} >> button.more")
    else:
        page.click(row, button="right")
    page.wait_for_selector("#ov-menu")


def row_path(page: Page, list_sel: str, idx: int) -> str:
    return page.get_attribute(f"{list_sel} .vrow:not([hidden]) >> nth={idx}", "data-path")


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


AUTO_KEYS = ["mode", "state", "reason", "policy", "session", "failed", "avg_full_s", "avg_drip_s", "eta_s", "current"]


def auto_fields(base: str, what: str) -> dict:
    """/api/cache's `auto` and /api/status's `cache`, keys in order."""
    r = get_json(base + "/api/cache")
    a = r.get("auto") or {}
    check(list(r)[-1:] == ["auto"] and list(a) == AUTO_KEYS, f"{what}: /api/cache auto {a}")
    c = get_json(base + "/api/status").get("cache") or {}
    check(list(c) == ["state", "prepared", "total"] and c["state"] in ("idle", "running", "paused", "done"),
          f"{what}: /api/status cache {c}")
    return a


def wait_auto_text(page: Page, pattern: str, timeout_ms: int = 20000) -> str:
    page.wait_for_function(f"new RegExp({json.dumps(pattern)}).test(document.querySelector('#pc-auto').textContent)", timeout=timeout_ms)
    return page.inner_text("#pc-auto")


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
        transition_checks(page, base)
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
        # Per-GIF overrides on the playlist in play (filelist.txt): favourite,
        # hide, play longer 2x, loop 3, on four distinct rows.
        fav_path = row_path(page, "#lib-list", 3)
        hide_path = row_path(page, "#lib-list", 4)
        long_path = row_path(page, "#lib-list", 5)
        loop_path = row_path(page, "#lib-list", 6)
        open_row_menu(page, "#lib-list", 3, via="more")
        page.click("#ov-fav")
        page.wait_for_selector("#ov-menu", state="detached")
        open_row_menu(page, "#lib-list", 4, via="right-click")
        page.click("#ov-hide")
        page.wait_for_selector("#ov-menu", state="detached")
        open_row_menu(page, "#lib-list", 5, via="more")
        page.click("#ov-2x")
        page.wait_for_selector("#ov-menu", state="detached")
        open_row_menu(page, "#lib-list", 6, via="more")
        page.fill("#ov-loops", "3")
        page.click("#ov-loops-set")
        page.wait_for_selector("#ov-menu", state="detached")
        check(page.locator("#lib-list .vrow:not([hidden]) >> nth=3 >> i.fav").count() == 1, "library overrides: favourite glyph")
        check(page.locator("#lib-list .vrow:not([hidden]) >> nth=4 >> i.hid").count() == 1
              and "hid" in (page.get_attribute("#lib-list .vrow:not([hidden]) >> nth=4", "class") or ""),
              "library overrides: hidden glyph + row class")
        check(page.locator("#lib-list .vrow:not([hidden]) >> nth=5 >> i.long").count() == 1, "library overrides: play-longer glyph")
        check(page.locator("#lib-list .vrow:not([hidden]) >> nth=6 >> i.loop").count() == 1, "library overrides: loop glyph")
        expected_over = [
            {"path": fav_path, "w": 3},
            {"path": hide_path, "never": True},
            {"path": long_path, "dwell_x": 2},
            {"path": loop_path, "loops": 3},
        ]
        r = wait_overrides(base, "", len(expected_over))
        check(r["overrides"] == expected_over, f"library overrides: GET /api/overrides?set= agrees ({r['overrides']})")
        meta_file = overlay / "filelist.meta"
        want_meta = "".join(render_meta_line(p["path"], p) + "\n" for p in expected_over)
        got_meta = meta_file.read_text() if meta_file.exists() else ""
        check(got_meta == want_meta, f"library overrides: filelist.meta byte-for-byte ({got_meta!r} vs {want_meta!r})")
        st = get_json(base + "/api/status")
        check(st["overrides"] == 4 and st["hidden"] == 1, f"library overrides: status overrides=4 hidden=1 (got {st['overrides']}/{st['hidden']})")
        no_leaks(page, "library (overrides menu)")
        shot(page, "override-menu", "laptop")
        # Phone: the same menu renders as a bottom sheet.
        phone = new_page(browser, "phone", errors)
        phone.goto(base + "/")
        phone.wait_for_function("document.querySelector('#now-path') && document.querySelector('#now-path').textContent.startsWith('/')", timeout=20000)
        goto_view(phone, "library")
        phone.wait_for_selector("#lib-cat option[value='Consoles']", state="attached", timeout=30000)
        phone.select_option("#lib-cat", "Consoles")
        phone.wait_for_function("document.querySelector('#lib-count').textContent.includes(' of ')", timeout=60000)
        open_row_menu(phone, "#lib-list", 0, via="more")
        check("sheet" in (phone.get_attribute("#ov-menu", "class") or ""), "phone: override menu renders as a bottom sheet")
        shot(phone, "override-menu", "phone")
        phone.context.close()
        # Reset them all: the file goes back to gone (an empty map deletes it).
        for idx in (3, 4, 5, 6):
            open_row_menu(page, "#lib-list", idx, via="more")
            page.click("#ov-reset")
            page.wait_for_selector("#ov-menu", state="detached")
        deadline = time.time() + 10
        while time.time() < deadline and (meta_file.exists() and meta_file.read_text().strip()):
            page.wait_for_timeout(100)
        check(not meta_file.exists() or not meta_file.read_text().strip(),
              "library overrides: reset all -> filelist.meta gone (or empty)")
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
        # Playlists: a /sets playlist's own overrides (the card copy has
        # none, so make one over the API first), favourited from the editor.
        ov_set_lines = want[:2]
        put_text(base + "/api/sets/overridetest", "\n".join(ov_set_lines) + "\n")
        # Made behind the browser's back: nudge it to reload the sets list
        # (it otherwise only does that on a config change).
        page.evaluate("async () => { const { app } = await import('./js/state.js'); await app.refreshConfig(); }")
        page.wait_for_selector("#sets-list [data-path='/sets/overridetest.txt']", timeout=20000)
        page.click("#sets-list [data-path='/sets/overridetest.txt'] >> text=Open")
        page.wait_for_selector("#set-entries .vrow[data-path]")
        ov_row_path = row_path(page, "#set-entries", 0)
        check(ov_row_path == ov_set_lines[0], f"playlists: editor opened overridetest ({ov_row_path})")
        open_row_menu(page, "#set-entries", 0, via="more")
        page.click("#ov-fav")
        page.wait_for_selector("#ov-menu", state="detached")
        r = wait_overrides(base, "overridetest", 1)
        check(r["overrides"] == [{"path": ov_row_path, "w": 3}], f"playlists: GET /api/overrides?set=overridetest agrees ({r['overrides']})")
        ov_meta_file = overlay / "sets" / "overridetest.meta"
        want_ov_meta = render_meta_line(ov_row_path, {"w": 3}) + "\n"
        got_ov_meta = ov_meta_file.read_text() if ov_meta_file.exists() else ""
        check(got_ov_meta == want_ov_meta, f"playlists: sets/overridetest.meta byte-for-byte ({got_ov_meta!r})")
        check(page.locator("#set-entries .vrow:not([hidden]) >> nth=0 >> i.fav").count() == 1, "playlists: favourite glyph in editor")
        no_leaks(page, "playlists (overrides)")
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
        # Auto prepare: playing with "idle" -> waiting; paused -> the mock's
        # simulated builder runs at full pace (it writes nothing).
        a = auto_fields(base, "auto")
        check(a["mode"] == "idle" and a["state"] == "paused" and a["reason"] == "playing", f"auto: playing -> paused ({a['state']}, {a['reason']})")
        t = wait_auto_text(page, "^Files are prepared automatically when the panel is idle · 0 of 3\\.")
        check(not page.is_checked("#pc-always"), f"auto: card reads '{t}', option unticked")
        try:
            put_json(base + "/api/config", {"auto_prepare": "sometimes"})
            check(False, "auto: a bad auto_prepare is refused")
        except urllib.error.HTTPError as e:
            check(e.code == 400 and "idle, always or off" in e.read().decode(), f"auto: a bad auto_prepare is a {e.code}")
        post_json(base + "/api/pause", {})
        goto_view(page, "sets")
        goto_view(page, "settings")
        t = wait_auto_text(page, "^Preparing files automatically while the panel is idle · [1-3] of 3 · about \\d+ s of idle time left\\.|^All files are prepared\\.")
        a = auto_fields(base, "auto (paused)")
        check(a["session"] > 0 and a["avg_full_s"] == 2.0, f"auto: paused -> prepared at full pace ('{t}', session {a['session']})")
        post_json(base + "/api/pause", {})
        page.click("#pc-go")
        page.wait_for_function("document.querySelector('#pc-progress').textContent.startsWith('3 files prepared')", timeout=120000)
        line = wait_cache_line(page, "3 of 3")
        check(True, f"cache: prepared -> '{line}' ({page.inner_text('#pc-progress')})")
        made = {bucketed(p): CARD / p.lstrip("/") for p in selected}
        compare_with_pack(overlay, made)
        no_leaks(page, "settings (prepared)")
        shot(page, "settings-cache", "laptop")
        t = wait_auto_text(page, "^All files are prepared\\.\\s*$")
        check(True, f"auto: after preparing, the card reads '{t}'")
        page.check("#pc-always")
        check(wait_config(base, lambda c: c.get("auto_prepare") == "always").get("auto_prepare") == "always", "auto: ticked -> auto_prepare always")
        page.locator("#pc").screenshot(path=str(OUT / "settings-auto-laptop.png"))
        page.uncheck("#pc-always")
        check(wait_config(base, lambda c: c.get("auto_prepare") == "idle").get("auto_prepare") == "idle", "auto: unticked -> auto_prepare idle")
        damage_checks(page, base, overlay, selected)
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
        studio_checks(page, base, overlay)
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
        # Time: pick a zone, save, and check the POSIX TZ string landed.
        page.select_option("#tm-zone", label="Europe/Dublin")
        page.click("#tm-save")
        page.wait_for_function("document.querySelector('#tm-now').textContent.includes('Europe/Dublin')", timeout=10000)
        cfg = get_json(base + "/api/config")
        check(cfg["tz"] == "GMT0IST,M3.5.0/1,M10.5.0" and cfg["tz_name"] == "Europe/Dublin",
              f"settings: time zone saved (tz={cfg['tz']!r}, tz_name={cfg['tz_name']!r})")
        # Schedule: a weekend rule with a playlist and a brightness.
        page.click("#sc-add")
        page.wait_for_selector("#sc-edit:not([hidden])")
        for day in (1, 2, 3, 4, 5):
            page.click(f"#sc-days .chip[data-day='{day}']")  # Mon-Fri default on -> off
        for day in (6, 7):
            page.click(f"#sc-days .chip[data-day='{day}']")  # Sat/Sun default off -> on
        page.fill("#sc-from", "10:00")
        page.fill("#sc-to", "12:00")
        page.select_option("#sc-set", value="filelist.hall")
        page.eval_on_selector("#sc-bright", "e => { e.value = 40; e.dispatchEvent(new Event('input')); }")
        page.click("#sc-rule-save")
        page.wait_for_selector("#sc-rules [data-rule='0']", timeout=10000)
        sched = get_json(base + "/api/schedule")
        rule = sched["rules"][0] if sched["rules"] else {}
        check(sorted(rule.get("days", [])) == [6, 7] and rule.get("from") == "10:00" and rule.get("to") == "12:00"
              and rule.get("set") == "filelist.hall" and rule.get("brightness") == 40,
              f"settings: schedule rule saved ({sched})")
        shot(page, "settings-schedule", "laptop")
        # Night mode covering right now: the Now banner should show it.
        now_dub = datetime.now(ZoneInfo("Europe/Dublin"))
        night_from = (now_dub - timedelta(minutes=1)).strftime("%H:%M")
        night_to = (now_dub + timedelta(minutes=30)).strftime("%H:%M")
        page.check("#sc-night-on")
        page.fill("#sc-night-from", night_from)
        page.fill("#sc-night-to", night_to)
        page.select_option("#sc-night-mode", value="off")
        page.click("#sc-night-save")
        page.wait_for_function("document.querySelector('#sc-night-on').checked", timeout=10000)
        sched = get_json(base + "/api/schedule")
        # schedule.js always sends the night brightness slider's value (8 by
        # default) alongside from/to/mode.
        check(sched.get("night") == {"from": night_from, "to": night_to, "mode": "off", "brightness": 8},
              f"settings: night mode saved ({sched.get('night')})")
        no_leaks(page, "settings (schedule)")
        # Phone: the schedule card and the Now banner.
        phone = new_page(browser, "phone", errors)
        phone.goto(base + "/")
        phone.wait_for_function("document.querySelector('#now-path') && document.querySelector('#now-path').textContent.startsWith('/')", timeout=20000)
        goto_view(phone, "settings")
        phone.wait_for_selector("#sc-rules [data-rule='0']", timeout=20000)
        shot(phone, "settings-schedule", "phone")
        goto_view(phone, "now")
        phone.wait_for_selector("#now-banner[data-mode='off']", timeout=15000)
        shot(phone, "now-banner", "phone")
        phone.context.close()
        # Now: the banner, waking for an hour, and resuming.
        goto_view(page, "now")
        page.wait_for_selector("#now-banner[data-mode='off']", timeout=15000)
        banner_text = page.inner_text("#now-banner")
        check("Night mode until" in banner_text and night_to in banner_text, f"now: night banner '{banner_text}'")
        st = get_json(base + "/api/status")
        check(st["mode"] == "off" and st["mode_until"] == night_to, f"now: status mode off until {st['mode_until']}")
        no_leaks(page, "now (night banner)")
        page.evaluate("window.scrollTo(0, 0)")
        shot(page, "now-banner", "laptop")
        page.click("#now-wake")
        page.wait_for_function("document.querySelector('#now-banner').textContent.includes('Schedule paused until')", timeout=15000)
        check(get_json(base + "/api/status")["mode"] == "normal", "now: woken -> status mode normal")
        page.click("#now-resume")
        page.wait_for_selector("#now-banner[data-mode='off']", timeout=15000)
        check(get_json(base + "/api/status")["mode"] == "off", "now: resumed -> back to night mode")
        # Clear the schedule: delete the rule, turn night mode off.
        goto_view(page, "settings")
        # The panel is off for the night: the builder runs, counted in nights.
        t = wait_auto_text(page, "^Preparing files automatically while the panel is idle · [\\d,]+ of [\\d,]+ · about \\d+ nights? left\\.|^All files are prepared\\.")
        check(True, f"auto: night mode -> '{t}'")
        no_leaks(page, "settings (auto)")
        page.click("#sc-rules [data-rule='0'] >> text=Delete")
        page.wait_for_selector("#sc-rules [data-rule]", state="detached", timeout=10000)
        page.uncheck("#sc-night-on")
        page.click("#sc-night-save")
        page.wait_for_function("!document.querySelector('#sc-night-on').checked", timeout=10000)
        sched = get_json(base + "/api/schedule")
        check(sched == {"rules": [], "night": None}, f"settings: schedule cleared -> empty form ({sched})")
        goto_view(page, "now")
        page.wait_for_selector("#now-banner", state="hidden", timeout=15000)
        check(get_json(base + "/api/status")["mode"] == "normal", "now: schedule cleared -> status mode normal")
        goto_view(page, "settings")
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
        follow_checks(page, base, errors)
        phone_pass(browser, base, errors)
    finally:
        proc.terminate()
    report_errors("mock", errors)


# --- The Studio ("Make art") ---------------------------------------------------

ART = REPO / "opendmd-art-web"
STUB_INDEX = {"repo": "Stub_-_Repo", "names": ["Other Game (Europe)", "Studio Test (Japan)", "Studio Test (USA)"],
              "kinds": "123"}


def studio_fixture() -> Path:
    """The CC0 test picture: the top 768x426 of a_source_windows.png (a 3x
    nearest-neighbour copy of a Kenney "Pixel Platformer" scene; the red and
    green boxes drawn on it start lower down). Plus its raw RGBA."""
    from PIL import Image
    src = REPO / "opendmd-web" / "experiments" / "art" / "process" / "out" / "a_source_windows.png"
    out = OUT / "studio-fixture.png"
    im = Image.open(src).convert("RGB").crop((0, 0, 768, 426))
    im.save(out)
    (OUT / "studio-fixture.rgba").write_bytes(im.convert("RGBA").tobytes())
    return out


def studio_expected(*opts: str) -> tuple[dict, bytes]:
    """The Rust pipeline's frames for the fixture: (its JSON line, raw RGB)."""
    rgb = OUT / "studio-expected.rgb"
    r = subprocess.run(["cargo", "run", "--release", "--quiet", "--example", "render", "--",
                        str(OUT / "studio-fixture.rgba"), "768", "426", str(rgb), *opts],
                       cwd=ART, check=True, capture_output=True, text=True, env={**os.environ, "RUSTUP_TOOLCHAIN": "stable"})
    return json.loads(r.stdout), rgb.read_bytes()


def gif_frames(path: Path) -> tuple[list[bytes], list[int]]:
    from PIL import Image
    im = Image.open(path)
    frames, delays = [], []
    for i in range(im.n_frames):
        im.seek(i)
        frames.append(im.convert("RGB").tobytes())
        delays.append(im.info.get("duration", 0))
    return frames, delays


def stub_art(page: Page, hits: list[str]) -> None:
    """A three-name SNES index, and libretro pictures served from the fixture;
    the USA snap is a link (a few bytes naming the Japan one), as some are."""
    png = (OUT / "studio-fixture.png").read_bytes()

    def raw(route) -> None:
        u = urllib.parse.unquote(route.request.url)
        hits.append(u)
        if "Named_Snaps/Studio Test (USA)" in u:
            route.fulfill(status=200, body="Studio Test (Japan).png", headers={"access-control-allow-origin": "*", "content-type": "text/plain"})
        else:
            route.fulfill(status=200, body=png, headers={"access-control-allow-origin": "*", "content-type": "image/png"})
    page.route("**/art/SNES.json", lambda r: r.fulfill(status=200, body=json.dumps(STUB_INDEX), headers={"content-type": "application/json"}))
    page.route("https://raw.githubusercontent.com/**", raw)


def open_studio(page: Page) -> None:
    goto_view(page, "library")
    page.wait_for_function("document.querySelector('#lib-count').textContent.includes(' of ')", timeout=60000)
    page.click("#lib-art")
    page.wait_for_selector("#studio:not([hidden])", timeout=20000)


def studio_data(page: Page) -> dict:
    return page.evaluate("({...document.querySelector('#studio').dataset})")


def studio_shots(page: Page, size: str) -> None:
    page.evaluate("document.querySelector('#studio').scrollTop = 0")
    shot(page, "studio", size)
    page.evaluate("document.querySelector('#studio').scrollTop = 1e6")
    page.wait_for_timeout(200)
    shot(page, "studio-save", size)
    page.evaluate("document.querySelector('#studio').scrollTop = 0")


def studio_checks(page: Page, base: str, overlay: Path) -> None:
    fixture = studio_fixture()
    hits: list[str] = []
    stub_art(page, hits)
    open_studio(page)
    # Find art for a game: the stubbed index, then a game's pictures.
    page.select_option("#st-find-plat", "SNES")
    page.fill("#st-find-q", "studio test")
    page.click("#st-find-go")
    page.wait_for_selector("#st-hits .st-hit")
    rows = page.eval_on_selector_all("#st-hits .st-hit", "els => els.map(e => e.textContent)")
    check(rows == ["Studio TestUSA", "Studio TestJapan"], f"studio find: 'studio test' on SNES -> {rows}")
    page.click("#st-hits .st-hit >> nth=0")
    page.wait_for_function("document.querySelectorAll('#st-thumbs .st-thumb:not([disabled])').length === 2", timeout=10000)
    kinds = page.eval_on_selector_all("#st-thumbs .st-thumb", "els => els.map(e => e.dataset.kind)")
    linked = any(h.endswith("Named_Snaps/Studio Test (Japan).png") for h in hits)
    check(kinds == ["Named_Titles", "Named_Snaps"] and linked,
          f"studio find: title and snap pictures load, the snap through its link ({kinds}, {len(hits)} requests)")
    page.click("#st-thumbs .st-thumb[data-kind=Named_Titles]")
    page.wait_for_function("document.querySelector('#studio').dataset.frames", timeout=20000)
    d = studio_data(page)
    check(d.get("platform") == "SNES" and d.get("title") == "Studio Test",
          f"studio find: a picked picture fills in platform and title ({d.get('platform')}, {d.get('title')})")
    # The same picture from a file: the upscale undone, Pixels, Pan.
    page.set_input_files("#st-file", str(fixture))
    page.wait_for_function("document.querySelector('#st-msg').textContent.startsWith('Picture: 768')", timeout=20000)
    page.click("#st-fit [data-v=pixels]")
    page.click("#st-motion [data-v=pan]")
    page.wait_for_timeout(400)
    exp, rgb = studio_expected("fit=pixels", "motion=pan")
    d = studio_data(page)
    check(d.get("scale") == "3" and d.get("stage") == f"{exp['stage'][0]}x{exp['stage'][1]}",
          f"studio frame: 3x found, stage {d.get('stage')} (pipeline {exp['stage']})")
    check([int(d["x"]), int(d["y"])] == exp["window"] and d.get("motion") == "pan" and int(d["frames"]) == exp["frames"],
          f"studio frame: auto band at {d.get('x')},{d.get('y')} == pipeline {exp['window']}, {d.get('frames')} frames")
    page.select_option("#st-plat", "SNES")
    page.fill("#st-title", "Studio Test")
    page.wait_for_timeout(500)
    name = "SNES_Studio_Test_01.gif"
    d = studio_data(page)
    check(d.get("name") == name, f"studio save: the name is {d.get('name')}")
    parsed = page.evaluate(f"import('./js/state.js').then(m => m.app.wasm.parsePath('/gifs/Consoles/{name}'))")
    check((parsed["platform"], parsed["title"], parsed["variant"], parsed["flags"]) == ("SNES", "Studio Test", 1, []),
          f"studio save: parsePath -> {parsed['platform']} / {parsed['title']!r} / {parsed['variant']} / {parsed['flags']}")
    check(f"“{parsed['title']} 01”" in page.inner_text("#st-name") and not page.inner_text("#st-warn"),
          f"studio save: shows the listed title ({page.inner_text('#st-name')})")
    # The grammar catch: a title word that is a tag is flagged before saving.
    page.fill("#st-title", "Toy Story")
    page.wait_for_timeout(300)
    check("“Toy”" in page.inner_text("#st-warn"), f"studio save: 'Toy Story' warned ({page.inner_text('#st-warn')})")
    page.fill("#st-title", "Studio Test")
    page.wait_for_timeout(500)
    no_leaks(page, "studio")
    studio_shots(page, "laptop")
    page.click("#st-save")
    page.wait_for_function("document.querySelector('#studio').dataset.saved", timeout=60000)
    page.wait_for_function("[...document.querySelectorAll('#toasts .toast.ok')].some(t => t.textContent.startsWith('1 added to “cachetest”'))", timeout=60000)
    path = f"/gifs/Consoles/{name}"
    got = overlay / path.lstrip("/")
    check(got.exists() and (overlay / bucketed(path).lstrip("/")).exists(),
          f"studio save: {path} and its prepared copy {bucketed(path)} landed")
    compare_with_pack(overlay, {bucketed(path): got})
    frames, delays = gif_frames(got)
    want = [rgb[i * 128 * 32 * 3:(i + 1) * 128 * 32 * 3] for i in range(exp["frames"])]
    same = sum(a == b for a, b in zip(frames, want))
    check(len(frames) == exp["frames"] and same == len(frames) and delays == exp["delays"],
          f"studio save: the GIF's {len(frames)} frames and delays equal the Rust pipeline's ({same} identical)")
    plist = (overlay / "sets" / "cachetest.txt").read_text().split("\n")
    check(path in plist, "studio save: appended to cachetest")
    page.click("#st-close")
    page.wait_for_selector("#studio", state="hidden")
    page.unroute("**/art/SNES.json")
    page.unroute("https://raw.githubusercontent.com/**")


def studio_device(page: Page) -> None:
    """Read only: the Studio opens and finds a game in the real SNES index;
    its pictures come from GitHub. Nothing is saved."""
    open_studio(page)
    page.select_option("#st-find-plat", "SNES")
    page.fill("#st-find-q", "super metroid")
    page.click("#st-find-go")
    page.wait_for_selector("#st-hits .st-hit", timeout=20000)
    rows = page.eval_on_selector_all("#st-hits .st-hit", "els => els.map(e => e.textContent)")
    check(any(r.startswith("Super Metroid") for r in rows), f"device studio: 'super metroid' -> {rows[:3]}")
    page.click("#st-hits .st-hit >> nth=0")
    try:
        page.wait_for_selector("#st-thumbs .st-thumb:not([disabled])", timeout=20000)
        check(True, f"device studio: pictures load ({page.locator('#st-thumbs .st-thumb').count()})")
    except PlaywrightTimeoutError:
        check(False, "device studio: no picture loaded from GitHub (network?)")
    no_leaks(page, "device studio")
    shot(page, "device-studio", "laptop")
    page.click("#st-close")


def damage_checks(page: Page, base: str, overlay: Path, selected: list[str]) -> None:
    """"Prepare remaining files" over a playlist `damagetest`: two of the
    selected GIFs (their copies forgotten) and two damaged GIFs written into
    the overlay (one cut short, one with a broken header). The first GET of
    one good GIF is aborted (Playwright route): it is retried after 2 s and
    made. The run ends in one summary line; the damaged GIFs are recorded in
    /odm/.failed (POST /api/cache/failed) and a second run never fetches
    them. cachetest is back in play, "3 of 3", afterwards."""
    good = selected[:2]
    src = CARD / good[0].lstrip("/")
    data = src.read_bytes()
    damaged = ["/gifs/Consoles/SNES_Smoke_Damaged_01.gif", "/gifs/Consoles/SNES_Smoke_Damaged_02.gif"]
    for p, b in zip(damaged, [data[: len(data) * 2 // 5], b"GIF00x" + data[6:]]):
        (overlay / p.lstrip("/")).parent.mkdir(parents=True, exist_ok=True)
        (overlay / p.lstrip("/")).write_bytes(b)
    for p in good:
        post_json(base + "/api/cache/forget", {"path": p})
    put_text(base + "/api/sets/damagetest", "".join(p + "\n" for p in good + damaged))
    put_json(base + "/api/config", {"set": "damagetest"})
    card = lambda u: urllib.parse.unquote(urllib.parse.urlparse(u).path)
    flaky, hits = good[1], []
    aborted = []
    def once(route):
        if not aborted:
            aborted.append(route.request.url)
            route.abort()
        else:
            route.continue_()
    match = lambda u: card(u) == flaky
    watch = lambda r: card(r.url) in damaged and hits.append(card(r.url))
    page.route(match, once)
    page.on("request", watch)
    # Chromium's console line for the aborted GET carries its URL.
    expected.append(urllib.parse.quote(base_name(flaky)))
    goto_view(page, "sets")
    goto_view(page, "settings")
    sync_app(page)
    wait_cache_line(page, "0 of 4")
    page.click("#pc-go")
    page.wait_for_function("(t => /^\\d+ files? prepared|^Every/.test(t))(document.querySelector('#pc-progress').textContent)", timeout=120000)
    line = page.inner_text("#pc-progress")
    check(line == "2 files prepared · 2 damaged files skipped (they can't be played either).",
          f"damage: summary line '{line}' (first GET of {base_name(flaky)} aborted: {len(aborted)})")
    check(len(aborted) == 1 and page.locator("#pc-net").count() == 0 and page.inner_text("#pc-errors") == "",
          "damage: the aborted download was retried, nothing listed as failed")
    page.click("#pc-damaged summary")
    listed = page.inner_text("#pc-damaged")
    check(all(p in listed for p in damaged) and page.inner_text("#pc-summary").strip().endswith("malformed GIF header"),
          f"damage: the disclosure lists both damaged GIFs, nothing else ({listed!r})")
    check(page.locator("#toasts .toast.bad").count() == 0, "damage: no error toasts")
    shot(page, "settings-damaged", "laptop")
    rows = get_json(base + "/api/cache/failed")
    check(sorted(r["path"] for r in rows) == damaged and all(r["reason"] for r in rows),
          f"damage: recorded in /odm/.failed ({rows})")
    compare_with_pack(overlay, {bucketed(p): CARD / p.lstrip("/") for p in good})
    n = len(hits)
    page.click("#pc-go")
    page.wait_for_function("(t => /^\\d+ files? prepared|^Every/.test(t))(document.querySelector('#pc-progress').textContent)", timeout=60000)
    line = page.inner_text("#pc-progress")
    check(line == "Every other file is already prepared · 2 damaged files skipped (they can't be played either)."
          and len(hits) == n, f"damage: second run '{line}', damaged GIFs fetched again: {len(hits) - n}")
    page.unroute(match)
    page.remove_listener("request", watch)
    expected.remove(urllib.parse.quote(base_name(flaky)))
    put_json(base + "/api/config", {"set": "cachetest"})
    req = urllib.request.Request(base + "/api/files/sets/damagetest.txt", method="DELETE")
    urllib.request.urlopen(req, timeout=30).read()
    for p in damaged:
        (overlay / p.lstrip("/")).unlink()
    sync_app(page)
    goto_view(page, "sets")
    goto_view(page, "settings")
    line = wait_cache_line(page, "3 of 3")
    check(True, f"damage: cachetest back in play, '{line}'")


def sync_app(page: Page) -> None:
    """The page catches up with a config change made behind its back."""
    page.evaluate("async () => { const { app } = await import('./js/state.js'); await app.refreshConfig(); await app.refreshStatus(); }")


def base_name(path: str) -> str:
    return path.rsplit("/", 1)[-1]


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


def follow_checks(page: Page, base: str, errors: list[str]) -> None:
    """Settings' "Follow RePlayOS" card and the Now line, against a fake
    RePlayOS (tools/mock-replay.py, started and stopped here)."""
    proc, replay_base, replay_port = start_replay("4321", "1")
    host_val = f"127.0.0.1:{replay_port}"
    try:
        print(f"\n== mock RePlayOS at {replay_base}")
        goto_view(page, "settings")
        page.wait_for_selector("#fo", timeout=10000)
        for sel in ("#fo-on", "#fo-host", "#fo-code", "#fo-attract", "#fo-test", "#fo-save"):
            check(page.locator(sel).count() == 1, f"follow: {sel} renders")
        page.wait_for_selector("#fo-attract option[value='*']", state="attached", timeout=10000)

        # Test: a wrong code, then the right one.
        page.fill("#fo-host", host_val)
        page.fill("#fo-code", "wrong")
        page.click("#fo-test")
        page.wait_for_function("!document.querySelector('#fo-result').textContent.includes('asking')", timeout=10000)
        t = page.inner_text("#fo-result")
        check("401" in t, f"follow: Test with a wrong code -> '{t}'")
        page.fill("#fo-code", "4321")
        page.click("#fo-test")
        page.wait_for_function("document.querySelector('#fo-result').textContent.includes('answered')", timeout=10000)
        t = page.inner_text("#fo-result")
        check("answered" in t, f"follow: Test with the right code -> '{t}'")

        # Save: follow on, host, code 4321, attract = the current playlist.
        page.check("#fo-on")
        page.fill("#fo-host", host_val)
        page.fill("#fo-code", "4321")
        page.select_option("#fo-attract", "*")
        page.click("#fo-save")
        cfg = wait_config(base, lambda c: c.get("follow") is True)
        raw_cfg = json.dumps(cfg)
        check(cfg.get("follow") is True and cfg.get("follow_host") == host_val
              and cfg.get("follow_code_set") is True and cfg.get("attract_set") is None
              and "4321" not in raw_cfg,
              f"follow: saved -> config {cfg}")
        try:
            page.wait_for_function("document.querySelector('#fo-code').value === ''", timeout=5000)
            cleared = True
        except PlaywrightTimeoutError:
            cleared = False
        check(cleared, f"follow: code field cleared after save (value: '{page.input_value('#fo-code')}')")
        page.wait_for_function(
            "(document.querySelector('#fo-code').placeholder || '').toLowerCase().includes('saved')", timeout=5000)
        ph = (page.get_attribute("#fo-code", "placeholder") or "").lower()
        check("saved" in ph, f"follow: code placeholder mentions a saved code ('{ph}')")
        no_leaks(page, "settings (follow)")

        # Now: the mock RePlayOS in the menu -> attract (the poller needs a
        # cycle to notice the game just enabled follow).
        replay_goto(replay_base, 0)
        goto_view(page, "now")
        page.wait_for_selector("#now-follow-line:not([hidden])", timeout=15000)
        page.wait_for_function(
            "document.querySelector('#now-follow-line').textContent.includes('attract')", timeout=15000)
        t = page.inner_text("#now-follow-line")
        check("Following RePlayOS" in t and "attract" in t, f"follow: menu -> Now line '{t}'")

        # Sonic 3 (step 1): "starting" at some point, then the game line;
        # while the title card is up, the title shows the game and the path
        # is empty, and nothing fetches the virtual path.
        reqs: list[str] = []

        def on_request(r):
            reqs.append(r.url)

        page.on("request", on_request)
        err_before = len(errors)
        replay_goto(replay_base, 1)
        saw_starting = False
        saw_title_card = False
        final_line = ""
        deadline = time.time() + 15
        while time.time() < deadline:
            line = page.inner_text("#now-follow-line")
            if "starting" in line:
                saw_starting = True
            if not saw_title_card:
                title_txt = page.inner_text("#now-title")
                path_txt = page.inner_text("#now-path")
                if "Sonic the Hedgehog 3" in title_txt and path_txt.strip() == "":
                    saw_title_card = True
                    check(True, f"follow: title card -> #now-title '{title_txt}', #now-path empty")
                    shot(page, "now-follow", "laptop")
            if "Mega Drive" in line and "Sonic the Hedgehog 3" in line and "matching" in line:
                final_line = line
                break
            page.wait_for_timeout(150)
        page.remove_listener("request", on_request)
        check(saw_starting or bool(final_line),
              f"follow: Sonic 3 -> saw 'starting' ({saw_starting}) or ended at '{final_line}'")
        check(saw_title_card, "follow: title card seen (title shown, path empty) during the starting sequence")
        at_reqs = [u for u in reqs if urllib.parse.urlsplit(u).path.startswith("/@")]
        check(not at_reqs, f"follow: no request for a virtual ('@...') path ({at_reqs})")
        check(len(errors) == err_before, f"follow: no console errors during the sequence ({errors[err_before:]})")
        if not final_line:
            wait_context(base, lambda c: c.get("state") == "game", timeout_s=10)
            final_line = page.inner_text("#now-follow-line")
        check("Mega Drive" in final_line and "Sonic the Hedgehog 3" in final_line and "matching" in final_line,
              f"follow: settled -> Now line '{final_line}'")

        # A wrong token: unreachable, with the 401 error, on both ends.
        replay_set_token(replay_base, "bogus")
        c = wait_context(base, lambda c: c.get("state") == "unreachable", timeout_s=25)
        check(c.get("state") == "unreachable" and c.get("error") == "401 (wrong code)",
              f"follow: wrong code -> context {c}")
        page.wait_for_function(
            "document.querySelector('#now-follow-line').textContent.toLowerCase().includes('not reachable')",
            timeout=15000)
        t = page.inner_text("#now-follow-line")
        check("not reachable" in t.lower(), f"follow: Now line says not reachable ('{t}')")
        replay_set_token(replay_base, "4321")
        no_leaks(page, "now (follow)")

        # Leave the mock as it started: not following, no host.
        goto_view(page, "settings")
        page.wait_for_selector("#fo-on", timeout=10000)
        if page.is_checked("#fo-on"):
            page.uncheck("#fo-on")
        page.fill("#fo-host", "")
        page.click("#fo-save")
        cfg = wait_config(base, lambda c: c.get("follow") is False)
        check(cfg.get("follow") is False and cfg.get("follow_host") == "",
              f"follow: disabled and host cleared before finishing ({cfg})")
        goto_view(page, "now")
    finally:
        proc.terminate()


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
    # The Studio on a phone: one column, nothing wider than the screen.
    hits: list[str] = []
    stub_art(page, hits)
    open_studio(page)
    page.set_input_files("#st-file", str(OUT / "studio-fixture.png"))
    page.wait_for_function("document.querySelector('#studio').dataset.frames", timeout=20000)
    page.select_option("#st-find-plat", "SNES")
    page.fill("#st-find-q", "studio")
    page.click("#st-find-go")
    page.wait_for_selector("#st-hits .st-hit")
    wide = page.evaluate("document.querySelector('#studio').scrollWidth - document.querySelector('#studio').clientWidth")
    check(wide <= 0, f"phone studio: no horizontal scroll ({wide}px)")
    no_leaks(page, "phone studio")
    studio_shots(page, "phone")
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
        cfg = get_json(base + "/api/config")
        check(cfg.get("transition") and page.input_value("#st-trans-ms") == str(cfg.get("transition_ms")),
              f"device settings: transition {cfg.get('transition')}:{cfg.get('transition_ms')} shown")
        st = get_json(base + "/api/status")
        facts = page.inner_text("#st-facts")
        check(st.get("index_state") in facts and (st.get("www_version") or "–") in facts,
              f"device settings: index_state {st.get('index_state')}, www_version {st.get('www_version')} shown")
        # Follow RePlayOS: read-only here -- the card renders and the API
        # shapes are present, but nothing is enabled or saved.
        follow_keys = ("follow", "follow_host", "follow_code_set", "attract_set")
        check(all(k in cfg for k in follow_keys), f"device config: follow keys present ({sorted(cfg)})")
        check("context" in st, f"device status: context present ({st.get('context')})")
        page.wait_for_selector("#fo", timeout=10000)
        check(page.locator("#fo-on").count() == 1 and page.locator("#fo-host").count() == 1
              and page.locator("#fo-save").count() == 1, "device settings: Follow RePlayOS card renders")
        line = wait_cache_line(page, f"{r['prepared']:,} of {r['total']:,}")
        check(True, f"device settings: cache card reads '{line}'")
        # Auto prepare, read only (the device's setting is left as it is).
        a = auto_fields(base, "device auto")
        check(cfg.get("auto_prepare") == a.get("mode"), f"device auto: auto_prepare {cfg.get('auto_prepare')}")
        t = wait_auto_text(page, "\\S")
        check(page.is_checked("#pc-always") == (a.get("mode") == "always"), f"device auto: card reads '{t}'")
        no_leaks(page, "device settings")
        shot(page, "device-settings", "laptop")
        studio_device(page)
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
