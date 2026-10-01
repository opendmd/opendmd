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
            three, byte-identical to `opendmd-pack convert`, and the
            card reads "3 of 3"
  Damaged   a playlist of two of them (copies forgotten) and two damaged
            GIFs written into the overlay (cut short, broken header); the
            first GET of one good GIF aborted (Playwright route) is retried:
            one summary line "2 files prepared · 2 damaged files skipped
            (they can't be played either).", both listed in the disclosure
            and in /api/cache/failed; a second run never fetches them
  Add GIFs  two GIFs uploaded into Consoles from Library: each GIF and its
            prepared copy land (the copy byte-identical to opendmd-pack), and
            the confirm adds both to `cachetest` ("5 of 5"); deleting one of
            the GIFs takes its copy with it. The names come from the Add GIFs
            dialog (#dl): SNES_Smoke_Added_01/_02 as picked
  Details   POST /api/files/move's 400/404/409; the Details panel of
            SNES_Mega_Man_X_08_sephirot68 (title, platform, category,
            author, every action shown); the Add GIFs dialog guessing
            "Super Metroid" / SNES from SNES_Super_Metroid_01.gif and naming
            it the next free number in Consoles (_11), no title for
            QT6JDNGFS3Q4OO6GBKO4LBW2ZZ4MO375.gif (Add disabled until given
            one), "Toy Story" refused, the name shown as typed; both
            uploaded and prepared; favourited in cachetest and listed in
            /sets/.smooth, then Edit details to another title and category:
            the GIF, its copy, the playlist line, the favourite and the
            Smooth motion line all follow; Edit in Make art loads it (first
            frame, title, platform); Delete takes it all away. Phone: the
            Details sheet opens and closes, no horizontal scroll
  Studio    "Make art" (a view: from Library's button, and the nav on a
            phone): a CC0 PNG (a crop of
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
  Find      Library -> GifCities against a canned answer (three shapes) and
            a 2-frame 88x31 GIF, both Playwright routes: the count, the
            Wide / Square-ish chips, size order, "Seen on", Open in Make art
            (2 frames, the title from the hit), a failed query's sentence;
            Your GIFs brings the list and its search back as they were
  Auto      preparing by the device (auto_prepare): /api/cache `auto` and
            /api/status `cache` keys; the card's sentence while playing
            ("…when the panel is idle · 0 of 3"), with the mock paused (its
            simulated builder at full pace, "about N s of idle time left")
            and in night mode ("about N nights left"), "All files are
            prepared" after preparing; the option ticked -> "always",
            unticked -> "idle"; a bad value is a 400; a shot of the card
  Smooth    Library's Smooth motion control: hidden for a GIF with
            nothing to smooth (OTHER_DBZ01), shown for a candidate
            (OTHER_SCROLL_Xmen01) with the analysis `opendmd-pack smooth
            analyse` prints ("N% ... adds M frames"); Preview plays the
            smooth version in the Library preview and back; on -> the line
            in /sets/.smooth and a marked file byte-identical to
            `opendmd-pack convert` with that list; overwritten with a plain
            one (what the device's builder would make after the cache was
            cleared), "Prepare remaining files" over a playlist of the two
            makes it smooth again; off -> the line gone and the plain file,
            byte-identical to opendmd-pack's; then analysis, tween and
            recode timed in wasm for MD_Demons_Of_Asteborg_01_sephirot68
            (4,489 frames) and OTHER_SCROLL_Xmen01
  Looks     Settings -> Playback's #st-look / #st-glow: Amber + Short saved
            show in /api/config and /api/status (glow_held 12288 while
            playing), the Now preview wears amber (every bright pixel r > g
            > b; now-amber-laptop.png), back to Full colour / Off; the bad
            look/glow/attract values are 400s
  Effects   Library -> Effects: the four effects preview and move, each with
            its options form, an option changes the row's path
            (library-effects-laptop.png); on the mock: Send to panel, POST
            /api/play of each (415 for an unknown one), attract "fx:..."
            validated, an @fx: playlist line survives a save and a reload
  Live      Library -> Live: five rows (four clocks and a scrolling text),
            the text form, a preview, the text row following #live-msg;
            Send to panel of the clock and the text (title, frame advancing;
            library-live-laptop.png); POST /api/play of live items 200, of
            @nonsense 415; the simulator (opendmd-sim --play
            '@text?msg=OpenDMD&scroll=0' --screenshot-canvas, default and
            --look amber) and a wasm Preview on a detached canvas draw the
            same 128x32 pixels (the device pass skips the simulator and
            sending; it does the read-only checks and the look round trip)
  Start-up  Settings -> Start-up (js/boot.js): #st-boot off/short/full shows
            the config's boot_show; saved as the other value and back (the
            device pass too: it restores it); "long", "" and 2 are 400s;
            Preview lights #st-boot-panel and moves, "No card screen" lights
            it; on the mock POST /api/play of @boot?len=card&.. plays as
            "Startup"; the simulator and the page draw that status card
            identically (settings-boot-laptop.png)
  Power     Settings -> Power (js/power.js): the Playback brightness slider's
            max follows max_brightness (also after a save); brightness above
            max_brightness is a 400 with the plain message, in the same request
            too; max outside 16..255 is a 400; lowering max lowers brightness in
            the same save (config and PUT reply carry both); the warning is
            hidden at/below brightness_safe_max and shown above it; the
            control saves and round-trips (mock only; settings restored)
  Web app   GET /api/www {version, bytes, entries, ok} (the version is
            www/version.txt, what the pack's header carries) and the same
            object as `www` in /api/status; PUT /api/www of the built pack
            is a 200, of something that is not a pack a 400; no firmware
            update card in Settings (OTA is gone)
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

Real-device pass (--device): first the web app as the device serves it
from its flash pack (GET /api/www verified, with the version this build
would write; when that version equals the local build's, every file the
device serves is byte-identical to what the pack holds — the `.gz` where
there is one, with `Content-Encoding: gzip`; the page loaded straight from
the device renders the Now view with no console errors; the misses and heap
across that part). Then the local build served through
`mock-device.py --proxy`, so nothing is written to the device's flash. Now
(and a minute of the preview's drift against the panel: median, max, RTT —
DRIFT_S=60 by default),
Library (+ a preview, sent to the panel with POST /api/play, which must show
up in /api/status; the Smooth motion control shown for OTHER_SCROLL_Xmen01,
nothing switched), a preview of a GIF that has a prepared copy (fetched from
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


def cache_path(gif_path: str) -> str:
    """/gifs/Cat/X.gif -> /cache/Cat/<xx>/X.gif (opendmd_core::layout)."""
    rel = gif_path.split("/", 2)[2]
    d, name = rel.rsplit("/", 1)
    return f"/cache/{d}/{fnv1a32(name.encode()) & 0xFF:02x}/{name}"


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
# preview asks for the prepared copy first, and a 404 there is normal; so
# is Smooth motion's list on a card that has none yet.
expected: list[str] = ["/cache/", "/sets/.smooth"]


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
    # And which load a connection closed with no response was (Chromium's
    # ERR_EMPTY_RESPONSE console line has no URL either).
    page.on("requestfailed", lambda r: "EMPTY_RESPONSE" in (r.failure or "")
            and print(f"  ..   [{size}] no response: {r.method} {r.url}"))
    page.on("dialog", lambda d: d.accept())
    return page


def pick_cat(page: Page, value: str) -> None:
    """Library: the category tab `value` (a directory under gifs/, `@` Live,
    `@fx` Effects)."""
    sel = f"#lib-cats [data-cat={json.dumps(value)}]"
    page.wait_for_selector(sel, timeout=30000)
    page.click(sel)


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
    pick_cat(page, "Consoles")
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
    # The base-playlist select fills when the root listing answers, which a
    # busy device can do late; "0 match" before that is not a count.
    page.wait_for_function("document.querySelectorAll('#b-base option').length > 0", timeout=30000)
    page.fill("#b-platforms", "MD,SNES")
    page.dispatch_event("#b-platforms", "change")
    page.wait_for_function("/^[1-9][\\d,]* match/.test(document.querySelector('#b-count').textContent)", timeout=30000)
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
    # A row re-rendered between press and release (the overrides loading)
    # swallows the click: try again once.
    for attempt in range(2):
        if via == "more":
            page.click(f"{row} >> button.more")
        else:
            page.click(row, button="right")
        try:
            page.wait_for_selector("#ov-menu", timeout=5000 if attempt == 0 else 30000)
            return
        except PlaywrightTimeoutError:
            if attempt:
                raise


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


# --- Looks and live items ---------------------------------------------------------

AMBER_PIXELS = """(sel) => {
    const c = document.querySelector(sel);
    const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
    let bright = 0, bad = 0;
    for (let i = 0; i < d.length; i += 4) {
        if (Math.max(d[i], d[i+1], d[i+2]) <= 80) continue;
        bright++;
        if (!(d[i] > d[i+1] && d[i+1] > d[i+2])) bad++;
    }
    return { bright, bad };
}"""

# A Preview built on a detached 128 px canvas (scale 1, flat: its pixels are the
# panel's), showing `path` after `look`; the 128x32 RGBA bytes as a list.
WASM_CANVAS = """async ([path, look]) => {
    const m = await import('./pkg/opendmd_web.js');
    const o = document.createElement('canvas');
    o.width = 128;
    const p = new m.Preview(o, 2.2, 1, 0.22, true);
    p.setTransition('none', 400);
    p.setDisplay(look, 'off');
    p.load(path, new Uint8Array());
    p.frame();
    const d = o.getContext('2d').getImageData(0, 0, 128, 32).data;
    p.free();
    return Array.from(d);
}"""


def put_status(url: str, obj) -> int:
    """PUT /api/config and return the HTTP status (200 or the error's)."""
    try:
        put_json(url + "/api/config", obj)
        return 200
    except urllib.error.HTTPError as e:
        return e.code


def look_validation(base: str) -> None:
    for bad in ({"look": "purple"}, {"glow": "medium"}, {"attract": "text:"}, {"attract": "bogus"}):
        check(put_status(base, bad) == 400, f"settings: {bad} refused (400)")


def save_look(page: Page, base: str, look: str, glow: str) -> None:
    """Settings -> Playback: choose a look and glow, Save playback; status
    and config must report them."""
    goto_view(page, "settings")
    page.wait_for_selector("#st-look", timeout=20000)
    page.select_option("#st-look", look)
    page.select_option("#st-glow", glow)
    page.click("#st-save-play")
    cfg = wait_config(base, lambda c: c.get("look") == look and c.get("glow") == glow)
    deadline = time.time() + 10
    st = get_json(base + "/api/status")
    while (st.get("look"), st.get("glow")) != (look, glow) and time.time() < deadline:
        time.sleep(0.2)
        st = get_json(base + "/api/status")
    check((cfg.get("look"), cfg.get("glow")) == (look, glow) and (st.get("look"), st.get("glow")) == (look, glow),
          f"settings: Save playback -> look {look}, glow {glow} in /api/config and /api/status "
          f"({st.get('look')}, {st.get('glow')}, glow_held {st.get('glow_held')})")


def look_checks(page: Page, base: str, mock: bool) -> None:
    """The Playback card's look and glow selects, their API validation, and
    the Now preview wearing the look. Leaves the original values saved."""
    goto_view(page, "settings")
    page.wait_for_selector("#st-look", timeout=20000)
    opts = page.eval_on_selector_all("#st-look option", "els => els.map(e => [e.value, e.textContent])")
    check(opts == [["colour", "Full colour"], ["amber", "Amber"], ["red", "Red"], ["green", "Green"], ["blue", "Blue"], ["white", "White"]],
          f"settings: #st-look options {opts}")
    check(page.eval_on_selector_all("#st-glow option", "els => els.map(e => e.value)") == ["off", "short", "long"],
          "settings: #st-glow options off/short/long")
    cfg0 = get_json(base + "/api/config")
    st0 = get_json(base + "/api/status")
    check(list(cfg0)[-4:] == ["attract", "look", "glow", "boot_show"],
          f"config: attract/look/glow/boot_show end the object ({list(cfg0)[-5:]})")
    check(list(st0)[-7:] == ["look", "glow", "glow_held", "attract_live", "max_brightness",
                             "brightness_safe_max", "safe_start_until"],
          f"status: look/glow/glow_held/attract_live/max_brightness/brightness_safe_max/safe_start_until"
          f" end the object ({list(st0)[-8:]})")
    ssu = st0["safe_start_until"]
    check(ssu is None or (isinstance(ssu, int) and ssu >= st0["uptime_secs"] - 1),
          f"status: safe_start_until is null or an uptime not yet passed ({ssu}, uptime {st0['uptime_secs']})")
    check(page.input_value("#st-look") == cfg0["look"] and page.input_value("#st-glow") == cfg0["glow"],
          f"settings: the selects show the config ({cfg0['look']}, {cfg0['glow']})")
    look_validation(base)
    orig = (cfg0["look"], cfg0["glow"])
    try:
        save_look(page, base, "amber", "short")
        st = get_json(base + "/api/status")
        if mock:
            check(st["glow_held"] == 12288 or st["paused"], f"status: glow short while playing holds 12288 B ({st['glow_held']})")
        goto_view(page, "now")
        wait_lit(page, "#now-panel", "now (amber)")
        page.wait_for_timeout(600)
        r = page.evaluate(AMBER_PIXELS, "#now-panel")
        check(r["bright"] > 100 and r["bad"] == 0,
              f"now: the preview wears amber (every one of {r['bright']} bright pixels has r > g > b; {r['bad']} do not)")
        shot(page, "now-amber", "laptop")
        # The tint is the page's (look), never a re-render of the file: colour restores it.
        save_look(page, base, "colour", "off")
        st = get_json(base + "/api/status")
        check(st["glow_held"] == 0, f"status: glow off holds nothing ({st['glow_held']})")
        goto_view(page, "now")
        page.wait_for_timeout(600)
        r = page.evaluate(AMBER_PIXELS, "#now-panel")
        check(r["bad"] > 0, f"now: back to full colour, the preview is no longer amber ({r['bad']} pixels off-hue)")
    finally:
        if (get_json(base + "/api/config")["look"], get_json(base + "/api/config")["glow"]) != orig:
            put_json(base + "/api/config", {"look": orig[0], "glow": orig[1]})
    cfg = get_json(base + "/api/config")
    check((cfg["look"], cfg["glow"]) == orig, f"settings: look/glow left as found {orig}")
    no_leaks(page, "settings (look)")
    goto_view(page, "now")


BOOT_CARD = "@boot?len=card&v=0.3.0&ip=192.168.1.69&note=Insert%20a%20microSD%20card"


def boot_checks(page: Page, base: str, mock: bool) -> None:
    """Settings -> Start-up (js/boot.js, imported by the view): the select and
    its round trip through /api/config (short -> full -> back), boot_show's
    validation, and the previews of the show and the no-card screen drawn in
    the page. On the mock, POST /api/play of @boot plays it as "Startup"."""
    goto_view(page, "settings")
    page.wait_for_selector("#st-boot", timeout=20000)
    opts = page.eval_on_selector_all("#st-boot option", "els => els.map(e => [e.value, e.textContent])")
    check(opts == [["off", "Off"], ["short", "Short"], ["full", "Full"]], f"settings: #st-boot options {opts}")
    cfg0 = get_json(base + "/api/config")
    orig = cfg0.get("boot_show")
    check(orig in ("off", "short", "full") and page.input_value("#st-boot") == orig,
          f"settings: #st-boot shows the config's boot_show ({orig})")
    for bad in ({"boot_show": "long"}, {"boot_show": ""}, {"boot_show": 2}):
        check(put_status(base, bad) == 400, f"settings: {bad} refused (400)")
    check(get_json(base + "/api/config")["boot_show"] == orig, "settings: a refused boot_show changed nothing")
    other = "full" if orig != "full" else "short"
    try:
        page.select_option("#st-boot", other)
        page.click("#st-save-boot")
        deadline = time.time() + 10
        while get_json(base + "/api/config")["boot_show"] != other and time.time() < deadline:
            time.sleep(0.2)
        check(get_json(base + "/api/config")["boot_show"] == other, f"settings: Save start-up -> boot_show {other}")
        page.select_option("#st-boot", orig)
        page.click("#st-save-boot")
        deadline = time.time() + 10
        while get_json(base + "/api/config")["boot_show"] != orig and time.time() < deadline:
            time.sleep(0.2)
    finally:
        if get_json(base + "/api/config")["boot_show"] != orig:
            put_json(base + "/api/config", {"boot_show": orig})
    check(get_json(base + "/api/config")["boot_show"] == orig, f"settings: boot_show left as found ({orig})")
    # The show, previewed: lit, and moving (two samples differ).
    page.select_option("#st-boot", "short")
    page.click("#st-boot-preview")
    wait_lit(page, "#st-boot-panel", "settings: start-up preview")
    a = canvas_sum(page, "#st-boot-panel")
    page.wait_for_timeout(300)
    check(a != canvas_sum(page, "#st-boot-panel"), "settings: the start-up preview moves")
    page.click("#st-boot-nocard")
    wait_lit(page, "#st-boot-panel", "settings: no-card screen preview")
    page.select_option("#st-boot", orig)
    shot(page, "settings-boot", "laptop")
    if mock:
        post_json(base + "/api/play", {"path": BOOT_CARD})
        st = get_json(base + "/api/status")
        check(st["now_playing"]["path"] == BOOT_CARD and st["now_playing"]["title"] == "Startup",
              f"play: @boot plays as 'Startup' ({st['now_playing']['path']}, {st['now_playing']['title']})")
        post_json(base + "/api/skip", {})
    no_leaks(page, "settings (start-up)")
    goto_view(page, "now")


def power_checks(page: Page, base: str) -> None:
    """Settings -> Power (js/power.js): the ceiling, its 400s, the slider and
    the warning. Mock only (it writes brightness and max_brightness, and puts
    them back)."""
    def cfg():
        return get_json(base + "/api/config")

    def slider_max():
        return int(page.eval_on_selector("#st-brightness", "e => e.max"))

    def reload_settings():
        # The page reads the config once; a PUT made behind its back needs a reload.
        goto_view(page, "settings")
        page.reload()
        page.wait_for_selector("#st-maxbright", state="attached", timeout=20000)

    orig = cfg()
    safe = get_json(base + "/api/status")["brightness_safe_max"]
    st = get_json(base + "/api/status")
    check(st["max_brightness"] == orig["max_brightness"] and isinstance(safe, int),
          f"power: status carries max_brightness {st['max_brightness']} and brightness_safe_max {safe}")
    try:
        put_json(base + "/api/config", {"max_brightness": safe, "brightness": 100})
        reload_settings()
        check(slider_max() == safe, f"power: the brightness slider's max is {safe} (follows max_brightness)")
        check(not page.is_visible("#st-maxbright"), "power: the control is behind the Advanced disclosure")
        page.click("#st-power summary")
        check(page.input_value("#st-maxbright") == str(safe), "power: the control shows max_brightness")
        check(not page.is_visible("#st-maxbright-warn"), "power: no warning at the tested limit")
        text = page.inner_text("#st-power-card")
        check(f"Above {safe} the panel can reset" in text and "especially on white screens" in text,
              "power: the explanation names the safe value")
        for jargon in ("brownout", "NVS", "rail"):
            check(jargon not in text, f"power: no jargon ({jargon})")
        # Refusals: nothing changes.
        r = put_status(base, {"brightness": safe + 1})
        check(r == 400, f"power: brightness above max is a 400 ({r})")
        c = cfg()
        check(c["brightness"] == 100 and c["max_brightness"] == safe, "power: a refused brightness changed nothing")
        req = urllib.request.Request(base + "/api/config", data=json.dumps({"brightness": 200}).encode(),
                                     method="PUT", headers={"Content-Type": "application/json"})
        try:
            urllib.request.urlopen(req)
            msg = ""
        except urllib.error.HTTPError as e:
            msg = e.read().decode()
        check(f"brightness 200 is above the maximum {safe}; raise Maximum brightness first" in msg,
              f"power: the 400 says what to do ({msg.strip()})")
        for bad in (15, 0, 256, 1000):
            check(put_status(base, {"max_brightness": bad}) == 400, f"power: max_brightness {bad} is a 400")
        check(put_status(base, {"max_brightness": 150, "brightness": 151}) == 400,
              "power: brightness is judged against the same request's max")
        check(cfg()["max_brightness"] == safe, "power: refused maximums changed nothing")
        # Raising both at once is fine, and shows the warning.
        r = put_json(base + "/api/config", {"max_brightness": safe + 40, "brightness": safe + 20})
        check(r.get("brightness") == safe + 20 and r.get("max_brightness") == safe + 40,
              f"power: PUT reply carries both ({r.get('brightness')}, {r.get('max_brightness')})")
        reload_settings()
        check(slider_max() == safe + 40, f"power: slider max follows a raised max_brightness ({slider_max()})")
        check(page.is_visible("#st-maxbright"), "power: the disclosure opens by itself above the tested limit")
        check(page.is_visible("#st-maxbright-warn"), "power: warning shown above the tested limit")
        w = page.inner_text("#st-maxbright-warn")
        check(w == f"Above the tested limit ({safe}). The display may reset. Only raise this with a stronger power supply.",
              f"power: warning text ({w})")
        shot(page, "settings-power", "laptop")
        # The control: lower the max below the brightness; both saved.
        page.fill("#st-maxbright", str(safe - 60))
        check(not page.is_visible("#st-maxbright-warn"), "power: warning hides when the control drops to the safe value")
        page.click("#st-save-power")
        deadline = time.time() + 10
        while cfg()["max_brightness"] != safe - 60 and time.time() < deadline:
            time.sleep(0.2)
        c = cfg()
        check(c["max_brightness"] == safe - 60 and c["brightness"] == safe - 60,
              f"power: lowering max lowers brightness in the same save ({c['brightness']}/{c['max_brightness']})")
        page.wait_for_function("+document.getElementById('st-brightness').max == %d" % (safe - 60), timeout=5000)
        check(int(page.input_value("#st-brightness")) == safe - 60, "power: the slider follows the lowered brightness")
        # Typing above the safe value shows the warning before saving.
        page.fill("#st-maxbright", str(safe + 1))
        check(page.is_visible("#st-maxbright-warn"), "power: warning shows as soon as the control is above the safe value")
        page.fill("#st-maxbright", str(safe))
        check(not page.is_visible("#st-maxbright-warn"), "power: warning hidden at the safe value")
        # Round trip through the Playback save too.
        page.eval_on_selector("#st-brightness", "e => { e.value = 40; e.dispatchEvent(new Event('input')); }")
        page.click("#st-save-play")
        page.wait_for_timeout(600)
        check(cfg()["brightness"] == 40 and cfg()["max_brightness"] == safe - 60, "power: Playback save leaves the maximum alone")
    finally:
        put_json(base + "/api/config", {"max_brightness": orig["max_brightness"]})
        put_json(base + "/api/config", {"brightness": orig["brightness"]})
    c = cfg()
    check((c["brightness"], c["max_brightness"]) == (orig["brightness"], orig["max_brightness"]),
          "power: brightness and maximum left as found")
    no_leaks(page, "settings (power)")
    goto_view(page, "now")


def open_live(page: Page) -> None:
    goto_view(page, "library")
    pick_cat(page, "@")
    # A search left over from the Library checks filters the Live rows too.
    page.fill("#lib-search", "")
    page.wait_for_selector("#lib-live #live-msg", timeout=10000)
    page.wait_for_function("document.querySelectorAll('#lib-list .vrow:not([hidden])').length >= 5", timeout=10000)


def live_rows(page: Page) -> list[str]:
    return page.eval_on_selector_all("#lib-list .vrow:not([hidden])", "els => els.map(e => e.dataset.path)")


def live_checks(page: Page, base: str, mock: bool) -> None:
    """Library -> Live: the clocks and the scrolling text; the form; a preview;
    on the mock, sent to the panel (and POST /api/play directly)."""
    goto_view(page, "library")
    page.wait_for_selector("#lib-cats [data-cat='@']", timeout=30000)
    opts = page.eval_on_selector_all("#lib-cats [data-cat]", "els => els.map(e => [e.dataset.cat, e.textContent])")
    check(opts[-2:] == [["@", "Live"], ["@fx", "Effects"]] and len(opts) > 2 and page.locator("#lib-cat").count() == 0,
          f"library: the category tabs end with Live, Effects, and there is no category select ({opts[-2:]})")
    open_live(page)
    rows = live_rows(page)
    want = ["@clock", "@clock?seconds=1", "@clock?date=1", "@clock?style=small&seconds=1&date=1", "@text?msg=OpenDMD"]
    check(rows == want, f"library: Live lists five rows ({rows})")
    check(page.is_visible("#lib-live") and page.input_value("#live-msg") == "OpenDMD"
          and page.eval_on_selector_all("#live-speed option", "els => els.map(e => e.value)") == ["60", "40", "25"],
          "library: Live form: #live-msg 'OpenDMD', #live-speed 60/40/25")
    page.click("#lib-list .vrow:not([hidden]) >> nth=0")
    wait_lit(page, "#lib-panel", "library live clock")
    cap = page.inner_text("#lib-d-title")
    check("clock" in cap.lower() and "@" not in cap, f"library: previewing the clock ('{cap}')")
    page.fill("#live-msg", "Hello smoke")
    page.dispatch_event("#live-msg", "change")
    # Leave the field now (its own change on blur redraws the list, which would
    # swallow the next click on a row).
    page.evaluate("document.activeElement.blur()")
    page.wait_for_timeout(400)
    page.wait_for_function("[...document.querySelectorAll('#lib-list .vrow')].some(e => e.dataset.path === '@text?msg=Hello%20smoke')", timeout=5000)
    check(live_rows(page)[-1] == "@text?msg=Hello%20smoke", f"library: the text row follows the field ({live_rows(page)[-1]})")
    wait_lit(page, "#lib-panel", "library live text")
    no_leaks(page, "library (live)")
    page.evaluate("window.scrollTo(0, 0)")
    shot(page, "library-live", "laptop")
    if not mock:
        page.fill("#live-msg", "OpenDMD")
        page.dispatch_event("#live-msg", "change")
        return
    expected.append("/api/play")
    for i, (path, title) in enumerate((("@clock", "Clock"), ("@text?msg=Hello%20smoke", "Hello smoke"))):
        page.click(f"#lib-list .vrow:not([hidden])[data-path='{path}']")
        page.wait_for_function(f"document.querySelector('#lib-play').title === {json.dumps(path)}", timeout=10000)
        page.click("#lib-play")
        cur = wait_playing(base, path)
        np_ = get_json(base + "/api/status")["now_playing"]
        check(cur == path and np_["title"] == title, f"library: Send to panel -> now_playing {path} '{np_['title']}'")
        f0 = np_["frame"]
        time.sleep(1.1)
        f1 = get_json(base + "/api/status")["now_playing"]["frame"]
        check(f1 > f0, f"status: {path} frame advances ({f0} -> {f1})")
    # The API directly.
    for path in ("@clock", "@text?msg=OpenDMD", "@clock?style=big&seconds=1&date=1&h12=1&color=FF8800",
                 "@text?msg=Hi&speed=25&font=small&color=00FF00&scroll=0"):
        try:
            r = post_json(base + "/api/play", {"path": path})
            check(r.get("ok") is True and r.get("path") == path, f"api: POST /api/play {path} -> 200")
        except urllib.error.HTTPError as e:
            check(False, f"api: POST /api/play {path} -> {e.code}")
    try:
        post_json(base + "/api/play", {"path": "@nonsense"})
        check(False, "api: POST /api/play @nonsense refused")
    except urllib.error.HTTPError as e:
        check(e.code == 415, f"api: POST /api/play @nonsense -> {e.code} (415)")
    expected.remove("/api/play")


FX_ROWS = ["@fx:tunnel", "@fx:rotozoom", "@fx:starfield", "@fx:wire"]


def effects_checks(page: Page, base: str, mock: bool) -> None:
    """Library -> Effects: the four effects, each previewing (lit, frames
    moving) with its options form; an option changes the row's path; on the
    mock, Send to panel, POST /api/play of each, and an effect's playlist
    line surviving a save and a reload."""
    goto_view(page, "library")
    pick_cat(page, "@fx")
    page.fill("#lib-search", "")
    page.wait_for_function("document.querySelectorAll('#lib-list .vrow:not([hidden])').length == 4", timeout=10000)
    rows = live_rows(page)
    check(rows == FX_ROWS, f"library: Effects lists the four effects ({rows})")
    titles = page.eval_on_selector_all("#lib-list .vrow:not([hidden]) .t", "els => els.map(e => e.textContent)")
    check(titles == ["Tunnel", "Rotozoom", "Starfield", "Wireframe"], f"library: effect titles {titles}")
    for i, path in enumerate(FX_ROWS):
        page.click(f"#lib-list .vrow:not([hidden]) >> nth={i}")
        name = path[4:]
        page.wait_for_selector(f"#lib-live #fx-{name}", timeout=5000)
        wait_lit(page, "#lib-panel", f"library effect {name}")
        a = canvas_sum(page, "#lib-panel")
        page.wait_for_timeout(400)
        b = canvas_sum(page, "#lib-panel")
        keys = page.eval_on_selector_all(f"#fx-{name} select", "els => els.map(e => e.dataset.key)")
        check(a != b and len(keys) >= 4, f"library: {name} previews and moves ({a} -> {b}); options {keys}")
    page.click("#lib-list .vrow:not([hidden]) >> nth=0")
    page.select_option("#fx-tunnel select[data-key='palette']", "fire")
    page.select_option("#fx-tunnel select[data-key='cycle']", "0")
    want = "@fx:tunnel?palette=fire&cycle=0"
    page.wait_for_function(f"[...document.querySelectorAll('#lib-list .vrow')].some(e => e.dataset.path === {json.dumps(want)})", timeout=5000)
    check(live_rows(page)[0] == want, f"library: the tunnel row follows its options ({live_rows(page)[0]})")
    wait_lit(page, "#lib-panel", "library effect tunnel (fire)")
    no_leaks(page, "library (effects)")
    page.evaluate("window.scrollTo(0, 0)")
    shot(page, "library-effects", "laptop")
    if not mock:
        return
    expected.append("/api/play")
    page.wait_for_function(f"document.querySelector('#lib-play').title === {json.dumps(want)}", timeout=10000)
    page.click("#lib-play")
    cur = wait_playing(base, want)
    np_ = get_json(base + "/api/status")["now_playing"]
    check(cur == want and np_["title"] == "Tunnel", f"library: Send to panel -> now_playing {cur} '{np_['title']}'")
    f0 = np_["frame"]
    time.sleep(0.5)
    f1 = get_json(base + "/api/status")["now_playing"]["frame"]
    check(f1 > f0 + 25, f"status: {want} frame advances at 100 fps ({f0} -> {f1})")
    for path in FX_ROWS + ["@fx:starfield?warp=0&density=3&fps=25", "@fx:wire?shape=tesseract&color=ff8800&fps=50"]:
        try:
            r = post_json(base + "/api/play", {"path": path})
            check(r.get("ok") is True and r.get("path") == path, f"api: POST /api/play {path} -> 200")
        except urllib.error.HTTPError as e:
            check(False, f"api: POST /api/play {path} -> {e.code}")
    try:
        post_json(base + "/api/play", {"path": "@fx:nonsense"})
        check(False, "api: POST /api/play @fx:nonsense refused")
    except urllib.error.HTTPError as e:
        check(e.code == 415, f"api: POST /api/play @fx:nonsense -> {e.code} (415)")
    expected.remove("/api/play")
    for bad, code in (({"attract": "fx:nonsense"}, 400), ({"attract": "fx:tunnel?palette=fire"}, 200)):
        check(put_status(base, bad) == code, f"settings: {bad} -> {code}")
    put_json(base + "/api/config", {"attract": ""})
    # A playlist with the effect's line: saved, read back, reloaded.
    page.click(f"#lib-list .vrow:not([hidden])[data-path='{want}'] input[type=checkbox]")
    page.click("#lib-to-set")
    page.wait_for_selector("#view-sets.on")
    page.evaluate(f"async () => {{ const {{ app }} = await import('./js/state.js'); const {{ emit }} = await import('./js/util.js'); app.draft.paths = [{json.dumps(want)}]; app.draft.name = ''; emit('draft'); }}")
    page.fill("#set-name", "fxsmoke")
    page.click("#set-save")
    page.wait_for_selector("#sets-list [data-path='/sets/fxsmoke.txt']", timeout=30000)
    with urllib.request.urlopen(base + "/sets/fxsmoke.txt", timeout=30) as resp:
        text = resp.read().decode()
    check(text.splitlines() == [want], f"playlists: sets/fxsmoke.txt holds the effect's line ({text!r})")
    page.evaluate("async () => { const { app } = await import('./js/state.js'); app.selection.clear(); }")
    page.reload()
    goto_view(page, "sets")
    page.click("#sets-list [data-path='/sets/fxsmoke.txt'] button.sm >> nth=0")
    page.wait_for_selector(f"#set-entries .vrow[data-path='{want}']", timeout=20000)
    t = page.inner_text(f"#set-entries .vrow[data-path='{want}'] .t")
    check(t.startswith("Tunnel"), f"playlists: after a reload the playlist lists the effect as '{t}'")
    no_leaks(page, "playlists (effects)")


def sim_png(root: Path, look: str, out: Path, path: str = "@text?msg=OpenDMD&scroll=0") -> bytes:
    """The desktop simulator's canvas for `@text?msg=OpenDMD&scroll=0` after
    `look`, as 128x32 RGBA bytes."""
    from PIL import Image
    (root / "gifs").mkdir(parents=True, exist_ok=True)
    cmd = ["cargo", "run", "--release", "-q", "--", "--root", str(root), "--play", path,
           "--screenshot-canvas", str(out)]
    if look != "colour":
        cmd += ["--look", look]
    r = subprocess.run(cmd, cwd=REPO / "opendmd-sim", env={**os.environ, "RUSTUP_TOOLCHAIN": "stable"},
                       capture_output=True, text=True, timeout=900)
    if r.returncode:
        raise RuntimeError(f"opendmd-sim failed:\n{r.stdout}\n{r.stderr}")
    im = Image.open(out).convert("RGBA")
    assert im.size == (128, 32), im.size
    return im.tobytes()


def sim_checks(page: Page) -> None:
    """A live text plays in the page's wasm Preview pixel for pixel as in the
    desktop simulator (default look, and amber)."""
    goto_view(page, "now")
    tmp = OUT / "sim"
    shutil.rmtree(tmp, ignore_errors=True)
    tmp.mkdir(parents=True)
    for look in ("colour", "amber"):
        want = sim_png(tmp / "root", look, tmp / f"sim-{look}.png")
        got = bytes(page.evaluate(WASM_CANVAS, ["@text?msg=OpenDMD&scroll=0", "" if look == "colour" else look]))
        lit_n = sum(1 for i in range(0, len(want), 4) if max(want[i:i + 3]) > 60)
        diff = sum(1 for i in range(0, len(want), 4) if want[i:i + 3] != got[i:i + 3])
        check(len(got) == len(want) and diff == 0 and lit_n > 50,
              f"sim: @text?msg=OpenDMD&scroll=0 ({look}) identical in the page and the simulator ({diff} of 4096 px differ, {lit_n} lit)")
    # The start-up show's status card: the same core in both.
    want = sim_png(tmp / "root", "colour", tmp / "sim-boot.png", BOOT_CARD)
    got = bytes(page.evaluate(WASM_CANVAS, [BOOT_CARD, ""]))
    lit_n = sum(1 for i in range(0, len(want), 4) if max(want[i:i + 3]) > 60)
    diff = sum(1 for i in range(0, len(want), 4) if want[i:i + 3] != got[i:i + 3])
    check(len(got) == len(want) and diff == 0 and lit_n > 200,
          f"sim: the start-up status card identical in the page and the simulator ({diff} of 4096 px differ, {lit_n} lit)")


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
        look_checks(page, base, True)
        boot_checks(page, base, True)
        power_checks(page, base)
        live_checks(page, base, True)
        effects_checks(page, base, True)
        sim_checks(page)
        post_json(base + "/api/skip", {})
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
        pick_cat(phone, "Consoles")
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
        made = {cache_path(p): CARD / p.lstrip("/") for p in selected}
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
        pick_cat(page, "Consoles")
        page.wait_for_function("document.querySelector('#lib-count').textContent.includes(' of ')", timeout=60000)
        page.set_input_files("#lib-add-files", [str(a) for a in adds])
        # The Add GIFs dialog: both named from their own names, the second
        # the next number; then Add.
        page.wait_for_function("(e => e && e.dataset.name === 'SNES_Smoke_Added_02.gif')(document.querySelector('#dl .dl-item[data-i=\"1\"]'))", timeout=20000)
        page.click("#dl-ok")
        page.wait_for_function("document.querySelector('#lib-add-progress').textContent.startsWith('2 of 2 added')", timeout=60000)
        page.wait_for_function("[...document.querySelectorAll('#toasts .toast.ok')].some(t => t.textContent.includes('cachetest'))", timeout=60000)
        gif_paths = [f"/gifs/Consoles/{a.name}" for a in adds]
        landed = [(overlay / p.lstrip("/")).exists() for p in gif_paths]
        check(all(landed), f"add: both GIFs uploaded ({landed})")
        compare_with_pack(overlay, {cache_path(p): a for p, a in zip(gif_paths, adds)})
        plist = (overlay / "sets" / "cachetest.txt").read_text().split("\n")
        check(plist[3:5] == gif_paths and get_json(base + "/api/status")["entries"] == 5,
              f"add: both appended to cachetest ({len([l for l in plist if l])} lines)")
        r = cache_ready(base)
        check((r["prepared"], r["total"]) == (5, 5), f"add: /api/cache {r['prepared']} of {r['total']}")
        no_leaks(page, "library (add)")
        shot(page, "library-add", "laptop")
        req = urllib.request.Request(base + "/api/files" + gif_paths[1].replace(" ", "%20"), method="DELETE")
        urllib.request.urlopen(req, timeout=30).read()
        gone = not (overlay / cache_path(gif_paths[1]).lstrip("/")).exists()
        r = cache_ready(base)
        check(gone and r["prepared"] == 4, f"add: deleting a GIF took its copy with it ({r['prepared']} of {r['total']})")
        details_checks(page, base, overlay)
        studio_checks(page, base, overlay)
        studio_anim_checks(page, base, overlay)
        find_checks(page)
        # Back to All files; the test playlist goes.
        goto_view(page, "sets")
        page.click("#sets-list [data-path='/sets/cachetest.txt'] >> text=Delete")
        page.wait_for_selector("#sets-list [data-path='/sets/cachetest.txt']", state="detached", timeout=60000)
        check(get_json(base + "/api/config")["set"] == "", "cache: cachetest deleted, All files playing")
        smooth_checks(page, base, overlay)
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
        www_mock_checks(page, base, version)
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
        views = page.eval_on_selector_all("#nav a[data-view]", "els => els.map(e => e.textContent.trim().slice(1))")
        check(views == ["Now", "Library", "Make art", "Playlists", "Settings"] and page.locator("#view-convert").count() == 0,
              f"nav: five views with Make art, no Convert ({views})")
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


def open_studio(page: Page, via: str = "#lib-art") -> None:
    """Make art: from the Library's header button, or the nav (`#nav-art`)."""
    if via == "#lib-art":
        goto_view(page, "library")
        page.wait_for_function("document.querySelector('#lib-count').textContent.includes(' of ')", timeout=60000)
    page.click(via)
    page.wait_for_selector("#view-studio.on #studio .st-grid", timeout=20000)
    check(page.url.endswith("#studio"), f"studio: {via} opens the Make art view ({page.url.rsplit('/', 1)[-1]})")


def studio_data(page: Page) -> dict:
    return page.evaluate("({...document.querySelector('#studio').dataset})")


def studio_shots(page: Page, size: str) -> None:
    page.evaluate("scrollTo(0, 0)")
    shot(page, "studio", size)
    page.evaluate("scrollTo(0, 1e6)")
    page.wait_for_timeout(200)
    shot(page, "studio-save", size)
    page.evaluate("scrollTo(0, 0)")


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
    # (The view keeps its picture between visits: forget the last one's frames.)
    page.evaluate("delete document.querySelector('#studio').dataset.frames")
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
    page.select_option("#st-cat", "Consoles")
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
    check(got.exists() and (overlay / cache_path(path).lstrip("/")).exists(),
          f"studio save: {path} and its prepared copy {cache_path(path)} landed")
    compare_with_pack(overlay, {cache_path(path): got})
    frames, delays = gif_frames(got)
    want = [rgb[i * 128 * 32 * 3:(i + 1) * 128 * 32 * 3] for i in range(exp["frames"])]
    same = sum(a == b for a, b in zip(frames, want))
    check(len(frames) == exp["frames"] and same == len(frames) and delays == exp["delays"],
          f"studio save: the GIF's {len(frames)} frames and delays equal the Rust pipeline's ({same} identical)")
    plist = (overlay / "sets" / "cachetest.txt").read_text().split("\n")
    check(path in plist, "studio save: appended to cachetest")
    goto_view(page, "library")
    page.unroute("**/art/SNES.json")
    page.unroute("https://raw.githubusercontent.com/**")


def anim_fixture(path: Path, n: int, w: int, h: int, delay: int) -> list[tuple[int, int, int]]:
    """An n-frame GIF: each frame a flat colour of its own with a yellow bar
    that moves (Pillow may store only what changed; the Studio composites).
    Returns the frames' colours."""
    from PIL import Image, ImageDraw
    cols = [((i * 53) % 200 + 30, (i * 97) % 200 + 30, (i * 31) % 200 + 30) for i in range(n)]
    frames = []
    for i, c in enumerate(cols):
        im = Image.new("RGB", (w, h), c)
        d = ImageDraw.Draw(im)
        x = (i * 7) % max(1, w - 8)
        d.rectangle([x, 0, x + 6, h // 3], fill=(250, 210, 30))
        d.ellipse([w - x - 18, h // 2, w - x - 2, h // 2 + 12], fill=(230, 60, 40))
        frames.append(im.quantize(64) if n > 20 else im.convert("P", palette=Image.Palette.ADAPTIVE, colors=16))
    frames[0].save(path, save_all=True, append_images=frames[1:], duration=delay, loop=0, optimize=False)
    return cols


def studio_timings(page: Page, where: str) -> None:
    """Import, a Look change (the frame shown made again) and a save (every
    frame rendered and the GIF written; then, apart, its copy prepared and
    both uploaded) of a 300-frame 256x224 GIF and a 50-frame 128x32 one."""
    for name, n, w, h, d in (("anim-300x256x224.gif", 300, 256, 224, 40), ("anim-50x128x32.gif", 50, 128, 32, 60)):
        f = OUT / name
        anim_fixture(f, n, w, h, d)
        page.evaluate("delete document.querySelector('#studio').dataset.tImport")
        page.set_input_files("#st-file", str(f))
        page.wait_for_function("document.querySelector('#studio').dataset.tImport", timeout=60000)
        t_import = int(studio_data(page)["tImport"])
        page.eval_on_selector("#st-gain", "e => { e.value = 90; e.dispatchEvent(new Event('input')); }")
        page.wait_for_timeout(400)
        t_look = float(studio_data(page)["tLook"])
        title = f"Timing {n} Frames"
        page.select_option("#st-plat", "OTHER")
        page.fill("#st-title", title)
        page.wait_for_function(f"document.querySelector('#studio').dataset.title === {json.dumps(title)}", timeout=10000)
        page.evaluate("delete document.querySelector('#studio').dataset.saved")
        page.click("#st-save")
        page.wait_for_function("document.querySelector('#studio').dataset.saved", timeout=120000)
        t_save, t_up = int(studio_data(page)["tSave"]), int(studio_data(page)["tUpload"])
        page.eval_on_selector("#st-gain", "e => { e.value = 100; e.dispatchEvent(new Event('input')); }")
        page.wait_for_timeout(300)
        check(int(studio_data(page)["srcFrames"]) == n, f"{where} studio timing {name}: {n} frames in")
        print(f"  studio timing {name}: import {t_import} ms, look change {t_look:.1f} ms (the frame shown), "
              f"save {t_save} ms (every frame rendered + written), then {t_up} ms to prepare + upload")


def studio_anim_checks(page: Page, base: str, overlay: Path) -> None:
    """An animated GIF in Make art: a 12-frame fixture loads with its frame
    count shown and the preview advancing; trim to frames 3-10, speed 2x,
    delete frame 6 (Undo, then again), text over it; saved, the GIF decodes
    to the 7 frames expected with 30 ms delays and the text, its prepared
    copy is in place and the Library lists it; Edit in Make art brings it
    back whole, Reverse and Replace the original rewrite the same file."""
    goto_view(page, "studio")
    fx = OUT / "anim-fixture.gif"
    cols = anim_fixture(fx, 12, 128, 32, 60)
    page.evaluate("delete document.querySelector('#studio').dataset.tImport")
    page.set_input_files("#st-file", str(fx))
    page.wait_for_function("document.querySelector('#studio').dataset.tImport", timeout=30000)
    d = studio_data(page)
    check(d.get("srcFrames") == "12" and "12 frames" in page.inner_text("#st-msg") and page.inner_text("#st-stats").startswith("12 frames · 0.7 s"),
          f"studio anim: 12 frames in, shown ('{page.inner_text('#st-msg')}', '{page.inner_text('#st-stats')}')")
    check(page.is_visible("#st-tl") and not page.is_visible("#st-motion"), "studio anim: the timeline shows, Pan/Scroll do not")
    p0 = studio_data(page).get("pos")
    page.wait_for_timeout(400)
    p1 = studio_data(page).get("pos")
    check(p0 != p1, f"studio anim: the preview plays (frame {p0} -> {p1})")
    cell = lambda i: f"#st-strip .st-cell[data-i='{i}']"
    page.click(cell(2))
    page.click("#st-set-in")
    page.click(cell(9))
    page.click("#st-set-out")
    page.select_option("#st-rate", "200")
    page.click(cell(5))
    page.keyboard.press("Delete")
    d = studio_data(page)
    check((d.get("from"), d.get("to"), d.get("frames"), d.get("duration"), d.get("deleted")) == ("2", "9", "7", "210", "1"),
          f"studio anim: trim 3-10, 2x, frame 6 deleted -> {d.get('frames')} frames, {d.get('duration')} ms")
    page.click("#st-undo")
    d = studio_data(page)
    check((d.get("frames"), d.get("deleted")) == ("8", "0"), f"studio anim: Undo brings frame 6 back ({d.get('frames')} frames)")
    page.click(cell(5))
    page.keyboard.press("Delete")
    page.fill("#st-text", "HI")
    page.wait_for_function("document.querySelector('#studio').dataset.text === '1'", timeout=5000)
    page.select_option("#st-plat", "SNES")
    page.fill("#st-author", "")
    page.fill("#st-title", "Smoke Frames")
    name = "SNES_Smoke_Frames_01.gif"
    page.wait_for_function(f"document.querySelector('#studio').dataset.name === '{name}'", timeout=10000)
    page.evaluate("document.querySelector('#st-preview').scrollIntoView()")
    page.wait_for_timeout(300)
    no_leaks(page, "studio (animation)")
    shot(page, "studio-anim", "laptop")
    page.evaluate("delete document.querySelector('#studio').dataset.saved")
    page.click("#st-save")
    page.wait_for_function("document.querySelector('#studio').dataset.saved", timeout=60000)
    path = f"/gifs/Consoles/{name}"
    got = overlay / path.lstrip("/")
    check(got.exists() and (overlay / cache_path(path).lstrip("/")).exists(), f"studio anim: {path} and its prepared copy landed")
    frames, delays = gif_frames(got)
    order = [2, 3, 4, 6, 7, 8, 9]
    firsts = [tuple(f[(10 * 128 + 127) * 3:(10 * 128 + 127) * 3 + 3]) for f in frames]
    texted = [sum(1 for i in range(24 * 128, 32 * 128) if f[i * 3:i * 3 + 3] == b"\xff\xff\xff") for f in frames]
    check(len(frames) == 7 and delays == [30] * 7 and firsts == [cols[i] for i in order] and all(t > 10 for t in texted),
          f"studio anim: saved GIF is frames {[i + 1 for i in order]} at 30 ms with the text ({len(frames)} frames, {delays}, text px {texted})")
    compare_with_pack(overlay, {cache_path(path): got})
    goto_view(page, "library")
    lib_row(page, path)
    check(page.inner_text("#lib-d-title") == "Smoke Frames 01", f"studio anim: listed in the Library ({page.inner_text('#lib-d-title')})")
    # Edit in Make art: the whole animation back; reversed, the original replaced.
    page.click("#lib-d-art")
    page.wait_for_function("(d => d.dataset.animated && d.dataset.srcFrames === '7')(document.querySelector('#studio'))", timeout=30000)
    check(page.input_value("#st-title") == "Smoke Frames" and page.is_visible("#st-mode [data-v=replace].on"),
          "studio anim: Edit in Make art brings the 7 frames, its title, Replace the original chosen")
    page.fill("#st-text", "")
    page.check("#st-rev")
    page.wait_for_function(f"document.querySelector('#studio').dataset.name === '{name}'", timeout=10000)
    page.evaluate("delete document.querySelector('#studio').dataset.saved")
    page.click("#st-save")
    page.wait_for_function("document.querySelector('#studio').dataset.saved", timeout=60000)
    frames2, delays2 = gif_frames(got)
    firsts2 = [tuple(f[(10 * 128 + 127) * 3:(10 * 128 + 127) * 3 + 3]) for f in frames2]
    same_dir = sorted(p.name for p in got.parent.iterdir() if p.name.startswith("SNES_Smoke_Frames"))
    check(firsts2 == firsts[::-1] and delays2 == [30] * 7 and same_dir == [name],
          f"studio anim: replaced in place, reversed ({same_dir})")
    compare_with_pack(overlay, {cache_path(path): got})
    goto_view(page, "studio")
    studio_timings(page, "mock")
    goto_view(page, "library")


# --- The Library's web sources (js/find.js) ------------------------------------

GC_HITS = [  # GifCities' JSON (gifcities.archive.org/api/v1/gifsearch): three shapes
    {"url_text": "spinning  mario 2", "weight": 90, "gif": "x/a.gif", "checksum": "SQUARECHECKSUM", "height": 64, "width": 64,
     "page": "https://web.archive.org/web/2009/http://geocities.com/a/index.html"},
    {"url_text": "cool mario banner 1", "weight": 50, "gif": "x/b.gif", "checksum": "WIDECHECKSUM", "height": 31, "width": 88,
     "page": "https://web.archive.org/web/2009/http://geocities.com/b/index.html"},
    {"url_text": "mario tower", "weight": 20, "gif": "x/c.gif", "checksum": "TALLCHECKSUM", "height": 120, "width": 50,
     "page": "https://web.archive.org/web/2009/http://geocities.com/c/index.html"},
]


def find_checks(page: Page) -> None:
    """Library -> GifCities against a canned answer and a 2-frame 88x31 GIF
    (Playwright routes; no network): the search, the count, the Wide chip
    narrowing to the one wide hit, "Seen on" the page it was on, Open in Make
    art loading it (2 frames, the title hint in Save); back to Your GIFs with
    the list and its search as they were."""
    from PIL import Image, ImageDraw
    gif = OUT / "find-fixture.gif"
    frames = []
    for i in range(2):
        im = Image.new("RGB", (88, 31), (20, 30, 120) if i == 0 else (120, 30, 20))
        ImageDraw.Draw(im).rectangle([4 + 40 * i, 6, 40 + 40 * i, 24], fill=(250, 210, 30))
        frames.append(im)
    frames[0].save(gif, save_all=True, append_images=frames[1:], duration=200, loop=0)
    asked: list[str] = []

    def api(route) -> None:
        asked.append(route.request.url)
        route.fulfill(status=200, body=json.dumps(GC_HITS), headers={"access-control-allow-origin": "*", "content-type": "application/json"})
    page.route("https://gifcities.archive.org/api/v1/gifsearch*", api)
    page.route("https://blob.gifcities.org/**", lambda r: r.fulfill(status=200, body=gif.read_bytes(), headers={"access-control-allow-origin": "*", "content-type": "image/gif"}))
    goto_view(page, "library")
    pick_cat(page, "Consoles")
    page.wait_for_function("document.querySelector('#lib-count').textContent.includes(' of ')", timeout=60000)
    page.fill("#lib-search", "mario")
    page.wait_for_timeout(300)
    before = (page.inner_text("#lib-count"), page.eval_on_selector_all("#lib-list .vrow:not([hidden]) .t", "els => els.slice(0, 5).map(e => e.textContent)"))
    page.click("#lib-src [data-src=gifcities]")
    page.wait_for_selector("#lib-find:not([hidden]) #find-narrow")
    check(not page.is_visible("#lib-list") and not page.is_visible("#lib-cats") and page.input_value("#lib-search") == ""
          and page.get_attribute("#lib-search", "placeholder") == "Search GifCities",
          "find: GifCities hides the list and the tabs; the box searches GifCities")
    page.fill("#lib-search", "mario")
    page.press("#lib-search", "Enter")
    page.wait_for_function("document.querySelector('#find-count').textContent === '3 of 3'", timeout=10000)
    check(len(asked) == 1 and "q=mario" in asked[0] and "limit=300" in asked[0], f"find: one query, 300 asked for ({asked})")
    dims = page.eval_on_selector_all("#find-grid .find-hit small", "els => els.map(e => e.textContent)")
    check(dims == ["64×64", "88×31", "50×120"], f"find: 3 of 3, best match first, sizes shown ({dims})")
    page.wait_for_function("[...document.querySelectorAll('#find-grid img')].every(i => i.complete && i.naturalWidth === 88)", timeout=10000)
    page.click("#find-shape [data-v=wide]")
    ids = page.eval_on_selector_all("#find-grid .find-hit", "els => els.map(e => e.dataset.id)")
    check(page.inner_text("#find-count") == "1 of 3" and ids == ["WIDECHECKSUM"], f"find: Wide narrows to the wide one ({page.inner_text('#find-count')}, {ids})")
    page.click("#find-shape [data-v=square]")
    ids = page.eval_on_selector_all("#find-grid .find-hit", "els => els.map(e => e.dataset.id)")
    check(ids == ["SQUARECHECKSUM"], f"find: Square-ish -> {ids}")
    page.click("#find-shape [data-v=any]")
    page.uncheck("#find-min")
    page.select_option("#find-sort", "size")
    ids = page.eval_on_selector_all("#find-grid .find-hit", "els => els.map(e => e.dataset.id)")
    check(ids == ["TALLCHECKSUM", "SQUARECHECKSUM", "WIDECHECKSUM"], f"find: biggest first -> {ids}")
    page.select_option("#find-sort", "rank")
    page.check("#find-min")
    page.click("#find-grid .find-hit[data-id=WIDECHECKSUM]")
    page.wait_for_selector("#find-pick:not([hidden])")
    check(page.get_attribute("#find-seen", "href") == GC_HITS[1]["page"] and "Cool Mario Banner 1" in page.inner_text("#find-pick"),
          f"find: the picked one, 'Seen on' its page ({page.inner_text('#find-pick')!r})")
    check("Results are unfiltered." in page.inner_text("#find-caption"), "find: the caption says results are unfiltered")
    no_leaks(page, "library (GifCities)")
    shot(page, "library-find", "laptop")
    page.evaluate("delete document.querySelector('#studio')?.dataset.tImport")
    page.click("#find-art")
    page.wait_for_selector("#view-studio.on", timeout=20000)
    page.wait_for_function("(d => d && d.dataset.tImport && d.dataset.srcFrames === '2')(document.querySelector('#studio'))", timeout=30000)
    msg = page.inner_text("#st-msg")
    check(msg.startswith("Cool Mario Banner 1: 88 × 31 · 2 frames") and page.input_value("#st-title") == "Cool Mario Banner",
          f"find: Open in Make art loads it ({msg!r}, title {page.input_value('#st-title')!r})")
    # A failed query: a sentence, the Library still there.
    expected.append("gifcities.archive.org")
    page.unroute("https://gifcities.archive.org/api/v1/gifsearch*")
    page.route("https://gifcities.archive.org/api/v1/gifsearch*", lambda r: r.abort())
    goto_view(page, "library")
    page.fill("#lib-search", "luigi")
    page.click("#lib-go")
    page.wait_for_function("document.querySelector('#find-status').textContent.includes('didn’t answer')", timeout=10000)
    check(page.inner_text("#find-count") == "", f"find: no answer -> '{page.inner_text('#find-status')}'")
    page.click("#lib-src [data-src='']")
    page.wait_for_selector("#lib-list", state="visible")
    after = (page.inner_text("#lib-count"), page.eval_on_selector_all("#lib-list .vrow:not([hidden]) .t", "els => els.slice(0, 5).map(e => e.textContent)"))
    check(after == before and page.input_value("#lib-search") == "mario" and page.is_hidden("#lib-find"),
          f"find: Your GIFs brings the list back as it was ({after[0]})")
    page.fill("#lib-search", "")
    page.unroute("https://gifcities.archive.org/api/v1/gifsearch*")
    page.unroute("https://blob.gifcities.org/**")


def move_api_checks(base: str) -> None:
    """POST /api/files/move's refusals (the mock mirrors http.rs)."""
    for body, code in (({"from": "/sets/x.txt", "to": "/gifs/Other/x.gif"}, 400),
                       ({"from": "/gifs/Other/a.gif", "to": "/gifs/Other/a.gif"}, 400),
                       ({"from": "/gifs/Other/no-such.gif", "to": "/gifs/Other/x.gif"}, 404),
                       ({"from": "/gifs/Consoles/SNES_Super_Metroid_02.gif", "to": "/gifs/Consoles/SNES_Super_Metroid_03.gif"}, 409)):
        try:
            post_json(base + "/api/files/move", body)
            got = 200
        except urllib.error.HTTPError as e:
            got = e.code
        check(got == code, f"move api: {body} -> {got} (want {code})")


def lib_row(page: Page, path: str) -> None:
    """Pick one GIF in the Library (its category, a search, the row)."""
    goto_view(page, "library")
    pick_cat(page, path.split("/")[2])
    page.wait_for_function("document.querySelector('#lib-count').textContent.includes(' of ')", timeout=60000)
    page.fill("#lib-search", path.rsplit("/", 1)[1][:-4])
    sel = f"#lib-list .vrow[data-path={json.dumps(path)}]"
    page.wait_for_selector(sel, timeout=20000)
    page.click(sel)
    page.wait_for_function(f"document.querySelector('#lib-cap').textContent === {json.dumps(path.rsplit('/', 1)[1])}", timeout=30000)


def details_checks(page: Page, base: str, overlay: Path) -> None:
    """The Details panel of a known GIF; Add GIFs' dialog (a title guessed
    from a name in the grammar, none from a hash-like one, which must be
    given one); Edit details renaming it through POST /api/files/move, its
    prepared copy, playlist line, favourite and Smooth motion line
    following; Edit in Make art; Delete taking all of it."""
    move_api_checks(base)
    known = "/gifs/Consoles/SNES_Mega_Man_X_08_sephirot68.gif"
    lib_row(page, known)
    got = {k: page.inner_text(f"#lib-d-{k}") for k in ("title", "platform", "category", "author")}
    check(got == {"title": "Mega Man X 08", "platform": "SNES", "category": "Consoles", "author": "sephirot68"},
          f"details: the fields of {known.rsplit('/', 1)[1]} ({got})")
    for sel in ("#lib-play", "#lib-d-add", "#lib-d-art", "#lib-d-fav", "#lib-d-hide", "#lib-ovset", "#lib-d-edit", "#lib-d-del"):
        check(page.is_visible(sel), f"details: {sel} shown")
    no_leaks(page, "library (details)")
    shot(page, "library-details", "laptop")
    # Add GIFs: a grammar name and a hash-like one.
    src = OUT / "add-named"
    shutil.rmtree(src, ignore_errors=True)
    src.mkdir(parents=True)
    small = CARD / "gifs" / "Consoles" / "SNES_Asterix01.gif"
    named, hashy = src / "SNES_Super_Metroid_01.gif", src / "QT6JDNGFS3Q4OO6GBKO4LBW2ZZ4MO375.gif"
    for f in (named, hashy):
        shutil.copy(small, f)
    have = [int(m.group(1)) for n in os.listdir(CARD / "gifs" / "Consoles")
            if (m := re.fullmatch(r"SNES_Super_Metroid_(\d+)\.gif", n))]
    want_named = f"SNES_Super_Metroid_{max(have, default=0) + 1:02d}.gif"
    pick_cat(page, "Consoles")
    page.set_input_files("#lib-add-files", [str(named), str(hashy)])
    page.wait_for_function("(e => e && e.dataset.name)(document.querySelector('#dl .dl-item[data-i=\"0\"]'))", timeout=20000)
    item = lambda i, sel: f"#dl .dl-item[data-i='{i}'] {sel}"
    t0, p0 = page.input_value(item(0, ".dl-title")), page.input_value(item(0, ".dl-plat"))
    n0 = page.get_attribute("#dl .dl-item[data-i='0']", "data-name")
    check((t0, p0, n0) == ("Super Metroid", "SNES", want_named),
          f"add dialog: {named.name} -> '{t0}' on {p0}, named {n0} (want {want_named}: the next free number)")
    t1 = page.input_value(item(1, ".dl-title"))
    err = page.inner_text(item(1, ".dl-err"))
    check(t1 == "" and "title" in err.lower() and page.is_disabled("#dl-ok"),
          f"add dialog: {hashy.name} -> no title guessed, one required ('{err}', Add disabled)")
    page.fill(item(1, ".dl-title"), "Toy Story")
    page.select_option(item(1, ".dl-plat"), "OTHER")
    page.wait_for_timeout(300)
    check("“Toy”" in page.inner_text(item(1, ".dl-err")) and page.is_disabled("#dl-ok"),
          f"add dialog: a title word read as a tag is refused ({page.inner_text(item(1, '.dl-err'))})")
    page.fill(item(1, ".dl-title"), "Smoke Test Card")
    page.fill(item(1, ".dl-author"), "Smoker")
    page.wait_for_function("document.querySelector('#dl .dl-item[data-i=\"1\"]').dataset.name === 'OTHER_Smoke_Test_Card_01_smoker.gif'", timeout=10000)
    check("OTHER_Smoke_Test_Card_01_smoker.gif" in page.inner_text(item(1, ".dl-name")) and page.is_enabled("#dl-ok"),
          f"add dialog: the name shown as typed ({page.inner_text(item(1, '.dl-name'))})")
    no_leaks(page, "add dialog")
    shot(page, "library-add-dialog", "laptop")
    page.evaluate("document.querySelectorAll('#toasts .toast').forEach(t => t.remove())")
    page.click("#dl-ok")
    page.wait_for_function("document.querySelector('#lib-add-progress').textContent.startsWith('2 of 2 added')", timeout=60000)
    added = ["/gifs/Consoles/" + want_named, "/gifs/Consoles/OTHER_Smoke_Test_Card_01_smoker.gif"]
    check(all((overlay / p.lstrip("/")).exists() and (overlay / cache_path(p).lstrip("/")).exists() for p in added),
          f"add dialog: both uploaded under their names, prepared ({added})")
    page.wait_for_function("[...document.querySelectorAll('#toasts .toast.ok')].some(t => t.textContent.startsWith('2 added to'))", timeout=30000)
    card = added[1]
    plist_file = overlay / "sets" / "cachetest.txt"
    check(card in plist_file.read_text().split("\n"), "add dialog: offered and appended to the playlist in play")
    # Mark it: a favourite in cachetest, Smooth motion's list.
    lib_row(page, card)
    page.select_option("#lib-ovset", "/sets/cachetest.txt")
    page.wait_for_timeout(500)
    page.click("#lib-d-fav")
    meta = overlay / "sets" / "cachetest.meta"
    deadline = time.time() + 10
    while time.time() < deadline and not (meta.exists() and card in meta.read_text()):
        page.wait_for_timeout(100)
    check(meta.exists() and meta.read_text().startswith(card + "\t"), "details: ★ Favourite written for cachetest")
    smooth = overlay / "sets" / ".smooth"
    smooth_before = smooth.read_text() if smooth.exists() else None
    smooth.parent.mkdir(parents=True, exist_ok=True)
    smooth.write_text((smooth_before or "") + card + "\n")
    # Edit details: a new title and another category -> a move.
    page.click("#lib-d-edit")
    page.wait_for_selector("#dl .dl-item[data-i='0'][data-name]", timeout=20000)
    f = {k: page.input_value(item(0, f".dl-{k}")) for k in ("title", "plat", "cat", "author")}
    check(f == {"title": "Smoke Test Card", "plat": "OTHER", "cat": "Consoles", "author": "smoker"}, f"edit: the fields read back ({f})")
    page.fill(item(0, ".dl-title"), "Smoke Renamed")
    page.select_option(item(0, ".dl-cat"), "Other")
    to = "/gifs/Other/OTHER_Smoke_Renamed_01_smoker.gif"
    page.wait_for_function(f"document.querySelector('#dl .dl-item[data-i=\"0\"]').dataset.name === {json.dumps(to.rsplit('/', 1)[1])}", timeout=10000)
    shot(page, "library-edit-dialog", "laptop")
    page.click("#dl-ok")
    page.wait_for_function(f"document.querySelector('#lib-cap').textContent === {json.dumps(to.rsplit('/', 1)[1])}", timeout=60000)
    check(not (overlay / card.lstrip("/")).exists() and (overlay / to.lstrip("/")).exists(), f"edit: renamed on the card to {to}")
    check(not (overlay / cache_path(card).lstrip("/")).exists() and (overlay / cache_path(to).lstrip("/")).exists(),
          f"edit: the prepared copy moved to {cache_path(to)}")
    lines_now = plist_file.read_text().split("\n")
    check(to in lines_now and card not in lines_now, "edit: the playlist line follows")
    check(meta.read_text().startswith(to + "\t"), "edit: the favourite follows")
    check(smooth.read_text().split("\n").count(to) == 1 and card not in smooth.read_text(), "edit: the Smooth motion line follows")
    check(page.inner_text("#lib-d-title") == "Smoke Renamed 01" and page.inner_text("#lib-d-category") == "Other",
          f"edit: the Details show the new fields ({page.inner_text('#lib-d-title')}, {page.inner_text('#lib-d-category')})")
    # Edit in Make art: the GIF — every frame — is the Studio's source.
    page.click("#lib-d-art")
    page.wait_for_function("(d => d && d.dataset.animated && d.dataset.srcFrames)(document.querySelector('#studio'))", timeout=30000)
    from PIL import Image
    want = Image.open(small).n_frames
    got = int(studio_data(page)["srcFrames"])
    check(got == want and f"{want} frames" in page.inner_text("#st-msg"),
          f"edit in make art: the whole animation loads ({got} frames, the file has {want}; '{page.inner_text('#st-msg')}')")
    check(page.input_value("#st-title") == "Smoke Renamed" and page.input_value("#st-plat") == "OTHER"
          and page.input_value("#st-author") == "smoker" and page.is_visible("#st-mode"),
          f"edit in make art: loaded with its fields ({page.input_value('#st-title')}, {page.input_value('#st-plat')}, "
          f"{page.input_value('#st-author')}) and Replace / Save as new")
    # Delete: after asking, the GIF, its copy, its playlist line, favourite, Smooth line.
    lib_row(page, to)
    page.click("#lib-d-del")
    page.wait_for_function("document.querySelector('#lib-d-body').hidden", timeout=60000)
    check(not (overlay / to.lstrip("/")).exists() and not (overlay / cache_path(to).lstrip("/")).exists(), "delete: the GIF and its copy are gone")
    check(to not in plist_file.read_text().split("\n"), "delete: the playlist line is gone")
    check(not meta.exists() or to not in meta.read_text(), "delete: the favourite is gone")
    check(to not in (smooth.read_text() if smooth.exists() else ""), "delete: the Smooth motion line is gone")
    # Leave /sets/.smooth as it was for the Smooth motion checks.
    if smooth_before is None:
        smooth.unlink(missing_ok=True)
    else:
        smooth.write_text(smooth_before)
    no_leaks(page, "library (edit, delete)")


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
    # An animated GIF from a file, played and edited here; nothing saved.
    fx = OUT / "anim-fixture.gif"
    anim_fixture(fx, 12, 128, 32, 60)
    page.evaluate("delete document.querySelector('#studio').dataset.tImport")
    page.set_input_files("#st-file", str(fx))
    page.wait_for_function("document.querySelector('#studio').dataset.tImport", timeout=30000)
    p0 = studio_data(page).get("pos")
    page.wait_for_timeout(400)
    d = studio_data(page)
    check(d.get("srcFrames") == "12" and d.get("pos") != p0, f"device studio: a 12-frame GIF loads and plays ({d.get('srcFrames')}, frame {p0} -> {d.get('pos')})")
    no_leaks(page, "device studio")
    shot(page, "device-studio", "laptop")
    goto_view(page, "library")


def damage_checks(page: Page, base: str, overlay: Path, selected: list[str]) -> None:
    """"Prepare remaining files" over a playlist `damagetest`: two of the
    selected GIFs (their copies forgotten) and two damaged GIFs written into
    the overlay (one cut short, one with a broken header). The first GET of
    one good GIF is aborted (Playwright route): it is retried after 2 s and
    made. The run ends in one summary line; the damaged GIFs are recorded in
    /cache/.failed (POST /api/cache/failed) and a second run never fetches
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
          f"damage: recorded in /cache/.failed ({rows})")
    compare_with_pack(overlay, {cache_path(p): CARD / p.lstrip("/") for p in good})
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
    subprocess.run(["cargo", "run", "--release", "--quiet", "--", "convert", str(mini), "--verify"],
                   cwd=REPO / "opendmd-pack", check=True, env={**os.environ, "RUSTUP_TOOLCHAIN": "stable"},
                   stdout=subprocess.DEVNULL)
    for copy in made:
        got = overlay / copy.lstrip("/")
        ref = mini / copy.lstrip("/")
        same = got.exists() and ref.exists() and filecmp.cmp(ref, got, shallow=False)
        size = got.stat().st_size if got.exists() else 0
        check(same, f"prepared copy {copy} byte-identical to opendmd-pack ({size} B)")


# --- Smooth motion -------------------------------------------------------------

SMOOTH_ON = "/gifs/Other/OTHER_SCROLL_Xmen01.gif"
SMOOTH_NONE = "/gifs/Other/OTHER_DBZ01.gif"
SMOOTH_WORST = "/gifs/Consoles/MD_Demons_Of_Asteborg_01_sephirot68.gif"


def pack(*args: str) -> str:
    return subprocess.run(["cargo", "run", "--release", "--quiet", "--", *args], cwd=REPO / "opendmd-pack",
                          check=True, env={**os.environ, "RUSTUP_TOOLCHAIN": "stable"},
                          capture_output=True, text=True).stdout


def pack_copy(gif: str, smooth: bool) -> bytes:
    """opendmd-pack convert's file for one GIF, on a card of its own, with
    or without it in /sets/.smooth."""
    mini = OUT / ("pack-smooth" if smooth else "pack-plain")
    shutil.rmtree(mini, ignore_errors=True)
    dst = mini / gif.lstrip("/")
    dst.parent.mkdir(parents=True)
    shutil.copy(CARD / gif.lstrip("/"), dst)
    if smooth:
        (mini / "sets").mkdir()
        (mini / "sets" / ".smooth").write_text(gif + "\n")
    pack("convert", str(mini), "--verify")
    return (mini / cache_path(gif).lstrip("/")).read_bytes()


def marked(b: bytes) -> bool:
    """The OPENDMD1TWN block right after the OPENDMD1LZW hint."""
    at = 13 + (3 * (2 << (b[10] & 7)) if b[10] & 0x80 else 0)
    return b[at + 3:at + 14] == b"OPENDMD1LZW" and b[at + 22:at + 33] == b"OPENDMD1TWN"


def preview_in_library(page: Page, gif: str) -> str:
    """Preview one GIF in Library; returns the Smooth motion box's state
    once the analysis is done."""
    goto_view(page, "library")
    cat, name = gif.split("/")[2], gif.rsplit("/", 1)[1]
    pick_cat(page, cat)
    page.wait_for_function("document.querySelector('#lib-count').textContent.includes(' of ')", timeout=60000)
    page.fill("#lib-search", name[:-4])
    page.wait_for_selector(f"#lib-list .vrow[data-path={json.dumps(gif)}]", timeout=20000)
    page.click(f"#lib-list .vrow[data-path={json.dumps(gif)}]")
    page.wait_for_function(f"document.querySelector('#lib-cap').textContent.endsWith({json.dumps(name)})", timeout=30000)
    page.wait_for_function("(s => s && s !== 'analysing')(document.querySelector('#lib-smooth').dataset.state)", timeout=120000)
    return page.get_attribute("#lib-smooth", "data-state")


def smooth_checks(page: Page, base: str, overlay: Path) -> None:
    listing = overlay / "sets" / ".smooth"
    copy = overlay / cache_path(SMOOTH_ON).lstrip("/")
    # What the CLI says about the two (percent to one decimal, frames added).
    mini = OUT / "pack-analyse"
    shutil.rmtree(mini, ignore_errors=True)
    for g in (SMOOTH_ON, SMOOTH_NONE):
        (mini / g.lstrip("/")).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy(CARD / g.lstrip("/"), mini / g.lstrip("/"))
    out = pack("smooth", "analyse", str(mini), "--min-percent", "10")
    rows = {m[3]: (float(m[0]), int(m[2])) for m in re.findall(r"^\s*([\d.]+)%\s+(\d+)\s+(\d+)\s+\d+\s+(\S.*)$", out, re.M)}
    check(SMOOTH_ON in rows and SMOOTH_NONE not in rows, f"smooth: opendmd-pack smooth analyse lists only the candidate ({rows})")
    pct, added = rows.get(SMOOTH_ON, (0.0, 0))
    # Nothing to smooth: no control at all.
    state = preview_in_library(page, SMOOTH_NONE)
    check(state == "none" and page.locator("#lib-smooth").is_hidden(), f"smooth: hidden for {SMOOTH_NONE} ({state})")
    # A candidate: the sentence with the CLI's numbers.
    state = preview_in_library(page, SMOOTH_ON)
    text = page.inner_text("#lib-smooth-text") if state in ("on", "off") else ""
    want = f"{int(pct)}% of this animation moves in steps of 2 px or more. Smooth motion adds {added:,} frames."
    check(state == "off" and page.locator("#lib-smooth").is_visible() and text == want,
          f"smooth: shown for {SMOOTH_ON}: '{text}' (analysis {page.get_attribute('#lib-smooth', 'data-analyse-ms')} ms)")
    no_leaks(page, "library (smooth)")
    shot(page, "library-smooth", "laptop")
    # Preview: the smooth version plays in the Library preview, then back.
    page.click("#lib-smooth-preview")
    page.wait_for_function("document.querySelector('#lib-cap').textContent.endsWith('smooth motion preview')", timeout=60000)
    wait_lit(page, "#lib-panel", "smooth preview")
    check(page.inner_text("#lib-smooth-preview") == "Show original", "smooth: Preview plays the smooth version")
    page.click("#lib-smooth-preview")
    page.wait_for_function(f"document.querySelector('#lib-cap').textContent.endsWith({json.dumps(SMOOTH_ON.rsplit('/', 1)[1])})", timeout=30000)
    check(page.inner_text("#lib-smooth-preview") == "Preview", "smooth: and back to the original")
    # On: the list line and a marked file, as opendmd-pack makes it.
    page.click("#lib-smooth-on")
    page.wait_for_function("document.querySelector('#lib-smooth').dataset.state === 'on'", timeout=120000)
    check(listing.exists() and listing.read_text() == SMOOTH_ON + "\n", f"smooth: on -> /sets/.smooth holds the line ({listing.read_text()!r})")
    ref = pack_copy(SMOOTH_ON, True)
    got = copy.read_bytes() if copy.exists() else b""
    check(marked(got) and got == ref, f"smooth: on -> marked file byte-identical to opendmd-pack's ({len(got)} B)")
    shot(page, "library-smooth-on", "laptop")
    # The device's builder replaces it with a plain one (after a cache
    # wipe): "Prepare remaining files" notices and makes it smooth again.
    plain = pack_copy(SMOOTH_ON, False)
    req = urllib.request.Request(base + "/api/files" + cache_path(SMOOTH_ON), data=plain, method="PUT")
    urllib.request.urlopen(req, timeout=60).read()
    check(copy.read_bytes() == plain, "smooth: the file replaced by a plain one")
    put_text(base + "/api/sets/smoothtest", f"{SMOOTH_ON}\n{SMOOTH_NONE}\n")
    put_json(base + "/api/config", {"set": "smoothtest"})
    sync_app(page)
    goto_view(page, "settings")
    page.click("#pc-go")
    page.wait_for_function("(t => /^\\d+ files? prepared|^Every/.test(t))(document.querySelector('#pc-progress').textContent)", timeout=120000)
    line = page.inner_text("#pc-progress")
    got = copy.read_bytes()
    check(line.startswith("1 file prepared") and got == ref, f"smooth: prepare run made it smooth again ('{line}', marked {marked(got)})")
    put_json(base + "/api/config", {"set": ""})
    req = urllib.request.Request(base + "/api/files/sets/smoothtest.txt", method="DELETE")
    urllib.request.urlopen(req, timeout=30).read()
    sync_app(page)
    # Off: the plain file again, made (not deleted), and the line gone.
    state = preview_in_library(page, SMOOTH_ON)
    check(state == "on" and page.is_checked("#lib-smooth-on"), f"smooth: shows as on ({state})")
    page.click("#lib-smooth-on")
    page.wait_for_function("document.querySelector('#lib-smooth').dataset.state === 'off'", timeout=120000)
    got = copy.read_bytes() if copy.exists() else b""
    check(listing.read_text() == "" and got == plain and not marked(got),
          f"smooth: off -> list emptied, plain file byte-identical to opendmd-pack's ({len(got)} B)")
    smooth_timings(page)


def smooth_timings(page: Page) -> None:
    """Analysis, tween and recode in wasm, each in one go (no slicing)."""
    for gif in (SMOOTH_WORST, SMOOTH_ON):
        r = page.evaluate("""async (path) => {
            const { app } = await import('./js/state.js');
            const m = await (await import('./js/artmod.js')).art();
            const bytes = await app.src.readBytes(path);
            const t0 = performance.now();
            const s = new m.Smoother(bytes);
            while (!s.analyse(100000)) {}
            const t1 = performance.now();
            while (!s.tween(100000)) {}
            const t2 = performance.now();
            const copy = s.prepare();
            const t3 = performance.now();
            const r = { frames: s.frames, tweened: s.tweened, added: s.added, percent: s.percent,
                        analyse: t1 - t0, tween: t2 - t1, recode: t3 - t2, bytes: bytes.length, copy: copy.length };
            s.free();
            return r;
        }""", gif)
        check(r["frames"] > 0, f"smooth timing {gif.rsplit('/', 1)[1]}: {r['frames']} frames, {r['percent']}% "
              f"(+{r['added']}), wasm analyse {r['analyse']:.0f} ms, tween {r['tween']:.0f} ms, recode {r['recode']:.0f} ms "
              f"({r['bytes']:,} -> {r['copy']:,} B)")


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
    # The Library's Details on a phone: a sheet over the list.
    goto_view(page, "library")
    check(not page.is_visible("#lib-side"), "phone library: no Details until a GIF is picked")
    page.click("#lib-list .vrow:not([hidden]) >> nth=0")
    page.wait_for_selector("#lib-side.open #lib-d-title", timeout=10000)
    wait_lit(page, "#lib-panel", "phone details")
    wide = page.evaluate("document.documentElement.scrollWidth - document.documentElement.clientWidth")
    check(page.is_visible("#lib-d-close") and wide <= 0, f"phone library: Details open as a sheet ({wide}px over)")
    shot(page, "library-details", "phone")
    page.click("#lib-d-close")
    check(not page.is_visible("#lib-side"), "phone library: the sheet closes")
    # GifCities on a phone (find_checks' canned answer and GIF).
    gif = (OUT / "find-fixture.gif").read_bytes()
    page.route("https://gifcities.archive.org/api/v1/gifsearch*", lambda r: r.fulfill(status=200, body=json.dumps(GC_HITS), headers={"access-control-allow-origin": "*", "content-type": "application/json"}))
    page.route("https://blob.gifcities.org/**", lambda r: r.fulfill(status=200, body=gif, headers={"access-control-allow-origin": "*", "content-type": "image/gif"}))
    page.click("#lib-src [data-src=gifcities]")
    page.wait_for_selector("#lib-find:not([hidden]) #find-narrow")
    page.fill("#lib-search", "mario")
    page.click("#lib-go")
    page.wait_for_function("document.querySelector('#find-count').textContent === '3 of 3'", timeout=10000)
    page.click("#find-grid .find-hit >> nth=1")
    wide = page.evaluate("document.documentElement.scrollWidth - document.documentElement.clientWidth")
    check(page.is_visible("#find-art") and wide <= 0, f"phone library: GifCities results, a hit picked ({wide}px over)")
    shot(page, "library-find", "phone")
    page.click("#lib-src [data-src='']")
    # The Studio on a phone, from the nav: one column, nothing wider than the screen.
    hits: list[str] = []
    stub_art(page, hits)
    open_studio(page, "#nav-art")
    page.set_input_files("#st-file", str(OUT / "studio-fixture.png"))
    page.wait_for_function("document.querySelector('#studio').dataset.frames", timeout=20000)
    page.select_option("#st-find-plat", "SNES")
    page.fill("#st-find-q", "studio")
    page.click("#st-find-go")
    page.wait_for_selector("#st-hits .st-hit")
    wide = page.evaluate("document.documentElement.scrollWidth - document.documentElement.clientWidth")
    check(wide <= 0, f"phone studio: no horizontal scroll ({wide}px)")
    no_leaks(page, "phone studio")
    studio_shots(page, "phone")
    page.context.close()


def www_mock_checks(page: Page, base: str, version: str) -> None:
    """GET/PUT /api/www against the mock; the OTA card is gone."""
    w = get_json(base + "/api/www")
    check(sorted(w) == ["bytes", "entries", "error", "ok", "version"] and w["ok"] and w["version"] == version
          and w["entries"] > 10, f"web app: GET /api/www {w}")
    st = get_json(base + "/api/status")
    check(st.get("www") == w and st.get("www_version") == version, "web app: /api/status carries www and www_version")
    pack = REPO / "opendmd-web" / "www.pack"
    for body, want in ((pack.read_bytes(), 200), (b"not a pack at all", 400)):
        req = urllib.request.Request(base + "/api/www", data=body, method="PUT",
                                     headers={"Content-Type": "application/octet-stream"})
        try:
            with urllib.request.urlopen(req, timeout=30) as r:
                code = r.status
        except urllib.error.HTTPError as e:
            code = e.code
        check(code == want, f"web app: PUT /api/www ({len(body):,} B) -> {code}")
    check(page.locator("#st-ota, #st-ota-go").count() == 0, "web app: no firmware update card in Settings")


def pack_files(www: Path) -> dict[str, tuple[Path, bool]]:
    """What `opendmd-pack www` puts in the pack, by served path: `<file>.gz`
    where there is one (gzip), else the file. version.txt is not packed."""
    out = {}
    for p in sorted(www.rglob("*")):
        rel = p.relative_to(www).as_posix()
        if not p.is_file() or p.name.startswith(".") or p.suffix == ".gz" or rel == "version.txt":
            continue
        gz = p.with_name(p.name + ".gz")
        out[rel] = (gz, True) if gz.exists() else (p, False)
    return out


def device_flash_ui(browser, device: str, errors: list[str]) -> None:
    """The web app straight from the device's flash (no proxy)."""
    print(f"\n== real device {device}: the web app from its flash pack")
    local = (REPO / "opendmd-web" / "www" / "version.txt").read_text().strip()
    st0 = get_json(device + "/api/status")
    w = get_json(device + "/api/www")
    check(w.get("ok") is True and w.get("version") and w.get("entries", 0) > 10,
          f"device web app: GET /api/www {w}")
    check(st0.get("www", {}).get("version") == w.get("version") and st0.get("www_version") == w.get("version"),
          f"device web app: /api/status www_version {st0.get('www_version')}")
    files = pack_files(REPO / "opendmd-web" / "www")
    if w.get("version") == local:
        check(w.get("entries") == len(files), f"device web app: {w.get('entries')} entries, the local build has {len(files)}")
        t0, same, total = time.time(), 0, 0
        for rel, (src, gz) in files.items():
            req = urllib.request.Request(f"{device}/{urllib.parse.quote(rel)}")
            with urllib.request.urlopen(req, timeout=30) as r:
                body, enc, clen = r.read(), r.headers.get("Content-Encoding"), r.headers.get("Content-Length")
            ok = body == src.read_bytes() and (enc == "gzip") == gz and clen == str(len(body))
            same += ok
            total += len(body)
            if not ok:
                check(False, f"device web app: /{rel} differs from the pack ({len(body)} B, encoding {enc})")
        dt = time.time() - t0
        check(same == len(files), f"device web app: {same}/{len(files)} files byte-identical to the pack "
                                  f"({total:,} B in {dt:.1f} s, {total / 1024 / max(dt, 1e-3):.0f} KB/s)")
        with urllib.request.urlopen(f"{device}/www/index.html", timeout=30) as r:
            check(r.read() == files["index.html"][0].read_bytes(), "device web app: /www/index.html is the same file")
    else:
        print(f"  (the device's pack is '{w.get('version')}', this build is '{local}': byte comparison skipped)")
    page = new_page(browser, "laptop", errors)
    t0 = time.time()
    page.goto(device + "/")
    page.wait_for_function("document.querySelector('#now-path') && document.querySelector('#now-path').textContent.startsWith('/')",
                           timeout=30000)
    check(page.locator("#now-stats .stat").count() >= 8,
          f"device web app: the page from flash renders the Now view ({time.time() - t0:.1f} s)")
    wait_lit(page, "#now-panel", "device web app", 40)
    goto_view(page, "settings")
    page.wait_for_selector("#st-facts .stat", timeout=20000)
    check(w.get("version", "?") in page.inner_text("#st-facts"), "device web app: Settings shows the pack's version")
    page.context.close()
    st1 = get_json(device + "/api/status")
    print(f"  device across the flash-UI checks: misses {st0['deadline_misses']} -> {st1['deadline_misses']}, "
          f"worst {st1['worst_lateness_ms']} ms, heap free {st1['heap']['free']} floor {st0['heap']['floor']} -> {st1['heap']['floor']}")


def device_pass(browser, device: str) -> None:
    errors: list[str] = []
    device_flash_ui(browser, device, errors)
    proc, base = start_mock(["--proxy", device])
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
        # Smooth motion, read only: the control for a candidate (on or off,
        # whatever the card's list says; nothing is switched here).
        state = preview_in_library(page, SMOOTH_ON)
        check(state in ("on", "off") and page.locator("#lib-smooth").is_visible(),
              f"device smooth: shown for {SMOOTH_ON} ({state}, analysis {page.get_attribute('#lib-smooth', 'data-analyse-ms')} ms)")
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
            pick_cat(page, cat)
            page.wait_for_function("document.querySelector('#lib-count').textContent.includes(' of ')", timeout=60000)
            for chip in page.query_selector_all("#lib-chips .chip.on"):
                chip.click()
            page.fill("#lib-search", name[:-4])
            page.wait_for_timeout(500)
            page.click(f"#lib-list .vrow:not([hidden]) >> nth=0")
            page.wait_for_function(f"document.querySelector('#lib-cap').textContent.endsWith({json.dumps(name[:-4] + '.gif')})", timeout=40000)
            wait_lit(page, "#lib-panel", f"device library {gif}", 40)
            fetched = [(u.split(base, 1)[-1], s) for u, s in reqs if "/cache/" in u or "/gifs/" in u]
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
        # Looks and live items, light: the API shapes, the validation (nothing
        # saved), the Playback card's round trip back to the original values,
        # and the Live list and preview (nothing sent to the panel).
        look_checks(page, base, False)
        boot_checks(page, base, False)
        live_checks(page, base, False)
        effects_checks(page, base, False)
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
