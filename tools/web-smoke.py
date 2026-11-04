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
  Library   open Consoles (#library/Consoles), search "mario" (only the
            folders with matches, open, counted), platform chip SNES,
            preview one, send it to the panel; no checkboxes until Select,
            then select three (a row tap picks in Select mode)
  Tree      a fresh browser, laptop and phone: the folders (Live, then
            system ones; no Effects), ⊟, the keys (↓ → → ↓ Enter ← ←), Consoles'
            3,781 GIFs in a few dozen rows with End reaching the last,
            #library/NEW%202021, Select's folder tick
  Thumbs    a fresh browser, the mock pacing card files (GET /mock/io
            ?kbps=400): the rows in view drawn, <= 2 requests in flight
            (page and mock), each read cut after the first frame (bytes
            per request against the whole copies), copies not GIFs, a
            fling fetching only where it stops, a reload fetching nothing
            (IndexedDB), a hover's clip (kept too), the switch off
  Editor    + Add GIFs in a new playlist: three ticked in Arcade (added in
            that order, ✓ in the tree), a folder's tick (Pixel_Art), 'mario'
            + SNES and Add all N shown (again: none added), thumbnails on
            the entries, Alt+↓, ✕, saved in order; the phone's panes take
            turns
  Sets      the playlists are listed by name (All files — the default —,
            Halloween, Christmas; never the obsolete filelist.odm.*);
            Play Christmas (set "filelist.xmas") and All files back as
            ""; + New playlist, Add by filter MD,SNES (Replace with N GIFs),
            save as `smoke` (the file must
            hold exactly the matching filelist.txt lines, in order), activate
            it, delete it (the UI switches back to the default first)
  Cache     the three selected GIFs as a playlist `cachetest`, activated;
            their prepared copies forgotten (POST /api/cache/forget), so the
            Settings card reads "0 of 3"; "Prepare remaining files" makes all
            three, byte-identical to `opendmd-pack convert`, and the
            card reads "3 of 3"
  Damaged   a playlist of two of them (copies forgotten), two damaged
            GIFs written into the overlay (cut short, broken header) and two
            of the card's GIFs past 256 colours (one with its copy
            forgotten, one keeping the card's lossy copy and a .failed line
            in earlier firmware's words); the first GET of one good GIF
            aborted (Playwright route) is retried: one summary line "4 files
            prepared · 2 damaged files skipped (they can't be played
            either)", the damaged ones listed in the disclosure and in
            /api/cache/failed, the two past 256 colours with no copy (the
            lossy one removed), recorded as playing as they are and counted
            as done ("4 of 6"); a second run fetches none of them. Then
            the first is replaced by a GIF of 256 colours or fewer (PUT, as
            Add GIFs and Replace do): its .failed line goes, its exact copy
            survives a run; with the stale line put back (an older player,
            a card edited elsewhere) a run keeps the copy too
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
            Smooth motion line all follow; Edit in Create Art loads it (first
            frame, title, platform); Delete takes it all away. Phone: the
            Details sheet opens and closes, no horizontal scroll
  Studio    "Create Art" (a view, from the nav; only step 1 Find until a
            picture is in, then Find folds to one line): a CC0 PNG (a crop of
            experiments/art/process/out/a_source_windows.png, a 3x copy of
            a Kenney scene) imported; the 3x found, Pixels, Pan, the auto
            band; saved as SNES_Studio_Test_01.gif: the GIF and its prepared
            copy land where they belong (the copy byte-identical to
            opendmd-pack's), the GIF's frames and delays equal the Rust
            pipeline's (`cargo run --example render` in opendmd-art-web),
            parsePath of the name gives SNES / "Studio Test" / 1, and it is
            appended to `cachetest`; Find's Game art: the name index is
            fetched from the project site (opendmd.dev/art-index/v1/,
            not the pack: /art/ is a 404), whose stand-in is a Playwright
            route serving opendmd-web/art-index/ with the CORS header (no
            network): unreachable -> the sentence and Try again, which then
            finds N64 games in the real index, fetched once for two
            searches; then a stubbed SNES index and stubbed libretro
            pictures (one of them a link to another); a click on a result
            loads it
  Studio    Smooth motion in Create Art: a 16-frame 128x32 GIF scrolling 4 px
  smooth    a frame (made with Pillow) loads, the switch on -> "Adds N
            frames across M moves" (N > 0) and the preview plays the
            smoothed frames; a speed change looks again; a GIF with nothing
            to smooth keeps the switch on and says so; saved as
            new -> listed in /sets/.smooth, the copy marked and
            byte-identical to opendmd-pack's with the list, Details shows
            it on; Edit in Create Art starts with the switch on, Reverse and
            Replace the original keep it listed and smooth; switched off and
            replaced -> unlisted, the plain copy (opendmd-pack's)
  Find      Create Art -> GifCities against a canned answer (three shapes)
            and a 2-frame 88x31 GIF, both Playwright routes: the count, the
            Wide / Square-ish chips, size order, a click loading the hit
            (2 frames, the title from the hit, "Seen on" its page, Find
            folded), Change picture, a failed query's sentence; the
            Library has no web sources, and a search that finds nothing
            there hands its words to Create Art
  Auto      preparing by the device (auto_prepare, default "idle"):
            /api/cache `auto` and
            /api/status `cache` keys; the card's sentence while playing
            ("…when the panel is idle · 0 of 3", plus the ETA if an
            earlier pause let it measure its pace), with the mock paused (its
            simulated builder at full pace, "about N s of idle time left")
            and in night mode ("about N nights left"), "All files are
            prepared" after preparing; the option ticked -> "always",
            unticked -> "idle"; a bad value is a 400; a shot of the card
  Smooth    Library's Smooth motion control: hidden for a GIF with
            nothing to smooth (OTHER_DBZ01), a sentence and no switch for
            a 1 px scroll (OTHER_SCROLL_The_Oluk: "already moves one pixel
            at a time"), shown for a candidate
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
  No effects  the built-in effects are gone: the Library has the Live folder and
            no Effects folder, tab or fx- form; on the mock POST /api/play of
            an @fx: path is a 415 ("Formulas replace them"), attract "fx:.." is
            a 400, an @fx: line in a playlist is skipped and the rest plays
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
  Web app   GET /api/www {version, bytes, entries, ok, error, staged} (the
            version is www/version.txt, what the pack's header carries;
            staged null before a PUT) and the same object as `www` in
            /api/status; PUT /api/www of the built pack is a 200 with staged
            true (kept by GET), of something that is not a pack a 400
  Firmware  a throwaway key pair (tools/fw-sign.py keygen; the mock started
            with --ota-pub) and a fake image (0xE9 + random bytes, a 4mb app
            descriptor) signed as 9.9.9-smoke, plus a copy with one image byte
            flipped, and an 8mb one. Settings -> Device -> Firmware shows GET
            /api/firmware's version and the board ("DevKit (4 MB)"); a file
            that is not a bundle is refused in the page, and so is the 8mb
            bundle, in words (the mock refuses its PUT too); the tampered bundle
            is refused by the mock (422, the reason shown, the version
            unchanged); the good one shows its progress, "Restarting…", then
            "Updated to 9.9.9-smoke" once /api/status reports it after the
            mock's simulated restart (its API a 503 for 3 s), the updater's
            `last` line shown, the "Update log" disclosure with the log's
            end (`log`), and About updated. A bundle older than the running
            version asks "… older than the firmware on the panel (X).
            Install it anyway?" (declined: nothing sent); `older()` itself
            on a table of version pairs
  Updates   Check for updates (js/update.js) against a fake project site on
            another port (release.json + files, `Access-Control-Allow-Origin:
            *` except under /nocors/): `wver()` on a table of web app
            versions; `?updates=` naming the running versions -> "Up to
            date"; a 404, no CORS header, not JSON, firmware ahead of the
            release -> a message and no Update button; a release naming a
            missing file, one whose 4mb entry (`variants`) is the 8mb bundle
            (refused in the page), and one whose firmware is tampered (the mock's
            422), install nothing and leave both versions alone; then
            firmware 10.0.0 + web app v10.0.0: each step shown, the mock
            restarts, PUT /api/www takes the pack, the page reloads, and a
            new check says "Up to date (10.0.0)"
  Security  the JSON routes without Content-Type application/json are 415
            (any case, parameters allowed; raw uploads take any type); a
            page from another origin (127.0.0.1:<another port>)
            POSTs /api/skip (no-cors fetch) and submits a form to /api/play:
            both 403, nothing skipped or played; the same Origin from a
            script is a 403, its own origin (any case, :80 dropped) is not.
            The API key (last): a bad one is a 400; Settings -> Device ->
            Security's switch makes a 24-character key and shows it once
            (not after a reload, the switch still on); then the API and card
            files need it (401 + WWW-Authenticate: Bearer without; the header
            or ?key= with; the app's own files stay open); a fresh browser
            gets the key dialog, a wrong key asks again, the right one loads
            Now; the switch off opens the API again
  Settings  four tabs at #settings/play|look|time|device, one in the page
            at a time; www_version and the status (no "index") shown; PUT
            brightness, read it back, Saved ✓ fading; WiFi scan; a hostname change is live (reboot_required
            false, status hostname/mdns at once; a WiFi change true) and
            Save network says where the display is now: no link from the
            IP; from smoke-a.local a link to smoke-b.local that loads, and
            to smoke-c-2.local on a (faked) clash; the Display name: a bad
            hostname is a 400, " Rig-Two " is saved as rig-two;
            /api/name-check (taken by an address, free, invalid, its own
            name) and a taken hostname a 409 with nothing saved; status
            `mdns` is the hostname, and with it rewritten to <name>-2
            (route) About shows that address and why; the setup page
            (/setup) prefills Display name, explains the clash, refuses a
            bad name before sending, and Connect sends the WiFi and the
            name together and names http://<name>.local/ (PUT and reboot
            caught), and a taken name (the mock's 409) is explained; in
            Network a taken name is said under the field and Save network
            sends nothing, and with the check faked to "couldn't tell" the
            409 is explained, nothing saved; the platforms row and sheet:
            counts most first, the rare behind +N more, the search, SNES
            and MD saved by Only these (nothing before), All platforms
            clears, platforms "Loo" (alone, or with a set and a
            brightness) a 400 naming it with nothing saved; Now with
            status rewritten to an empty playlist names the filter and
            Clear filter clears it; the playlist choices are labelled; with
            /api/status rewritten to index_state "building" (Playwright
            route), set switching is disabled in Settings and Sets, and comes
            back when it reads "ready"; About's "API reference" links the
            HTTP API on the project site in a new tab: a release's own page
            (dev/api/v<version>.html) when the status says firmware and web
            app are that release, else (this build, `git describe`; a
            version that is no release number) dev/api.html; no api.html in
            the www pack, and the mock serves none

Every view's visible text (and every title tooltip) is checked for "odm" and
"bucket": prepared copies are an implementation detail.

Unreadable-card pass: a second mock with `--sd unreadable` (a card that
mounts but cannot be read, exFAT): status `sd` {mounted: true, readable:
false, error naming FAT32}, the Now page's Card line "Not readable: needs
FAT32" in red and its "Format card…" line, and PUT /api/www replying
staged false. Then the format: POST /api/card/format without `confirm` is
a 400; the dialog (warning, red button) cancelled changes nothing; Format
card shows progress with `sd.formatting` true (a second POST: 409), ends
"Done" with the card readable, and a format of the now readable card is a
409.

Slowed-card pass: a third mock with `--sd slowed` (a card set to Fast
that has already been slowed in place): status sd {clock_khz 20000,
fallback, slowdowns 1, probe "slowed"}; Settings › Device shows the Card
speed group (#st-cs, Fast), "Card: ... Safe speed", and the slow line
(#st-card-slow); "Try Fast again" hides it and the Card line reads Fast
speed (status 40 MHz). Then the select (Safe and Fast only): Safe (Saved
tick, config safe, status 20 MHz without fallback, its caption,
POST /api/card/full-speed a 409), Fast (40 MHz, its caution); `full` over
the API is accepted, `auto` is a 400.

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

www update pass (also alone: --www-update-only): the app is served as two
builds whose wasm modules and glue are marked (tools/www-build-id.py
writes each one's js/build.js); the first with 1.0.0-beta.13's headers
(the wasm kept an hour), then the files replaced by the second and a plain
reload: every wasm module, and its glue, must be the second build's (the
beta.13 -> beta.14 failure: an old Psi compiler run by a new app). Also:
a wasm file goes out no-cache with an ETag, and 304 for it.

Screenshots of every view at 1280x800 and 390x844 go to target/web-smoke/.
Any console error or page error fails the run.
"""
import argparse
import atexit
import base64
import filecmp
import gzip
import http.server
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
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


def card_gifs() -> list[str]:
    """The playlist's GIFs (filelist.txt) that the card copy really holds:
    the dump lists a few it does not, and a check that plays or fetches one
    would meet a 404 (and a console error)."""
    lines = (CARD / "filelist.txt").read_text(encoding="utf-8-sig").split("\n")
    return [l.strip() for l in lines if l.strip().endswith(".gif") and (CARD / l.strip().lstrip("/")).is_file()]
OUT = Path(os.environ.get("WEB_SMOKE_OUT", REPO / "target" / "web-smoke"))
# Create Art's game name index, published on the site (tools/site.py); every
# page's context answers it from the committed files (site_art_index), so no
# test reaches the network. Requests seen: [(url, Origin header)].
SITE_ART = "https://opendmd.dev/art-index/v1/"
ART_INDEX = REPO / "opendmd-web" / "art-index"
site_art_asked: list[tuple[str, str | None]] = []
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


_www_snap: Path | None = None


def www_snapshot() -> Path:
    """The web app as it was when the run began, copied once: the mock serves
    this, not opendmd-web/www. `make web` and `make site` rewrite that
    directory (delete the .gz files, replace pkg/), and a page that loads
    its modules in that moment gets a 404 and never draws: a build running
    beside the smoke made a bare wait time out. (A reload would not do:
    the page under test is the thing being measured.)"""
    global _www_snap
    if _www_snap is None:
        OUT.mkdir(parents=True, exist_ok=True)
        _www_snap = Path(tempfile.mkdtemp(prefix="www-snap-", dir=OUT)) / "www"
        shutil.copytree(REPO / "opendmd-web" / "www", _www_snap)
        if (REPO / "opendmd-web" / "www.pack").is_file():  # beside it, where the mock looks (www_info)
            shutil.copy2(REPO / "opendmd-web" / "www.pack", _www_snap.parent / "www.pack")
        atexit.register(shutil.rmtree, _www_snap.parent, True)
    return _www_snap


mock_logs: list[Path] = []


def mock_errors() -> None:
    """Every mock's unhandled errors (each answered 500), with tracebacks."""
    hits = []
    for log in mock_logs:
        text = log.read_text(errors="replace") if log.is_file() else ""
        hits += [f"{log.name}: {blk.strip()}" for blk in text.split("mock-device: ")[1:] if blk.startswith("unhandled error")]
    check(not hits, f"mock: no unhandled errors (500s) in {len(mock_logs)} mock runs")
    for h in hits:
        print("  !!   " + h.replace("\n", "\n  !!   "))


def start_mock(args: list[str]) -> tuple[subprocess.Popen, str]:
    port = free_port()
    if "--www" not in args:
        args = [*args, "--www", str(www_snapshot())]
    # Its output (with --quiet: the banner and any unhandled error's
    # traceback, each a 500 to the page) goes to a file, read back by
    # mock_errors() at the end: a 500 is a bug in the mock, never noise.
    OUT.mkdir(parents=True, exist_ok=True)
    log = OUT / f"mock-{len(mock_logs)}.log"
    mock_logs.append(log)
    with log.open("wb") as f:
        proc = subprocess.Popen(
            ["uv", "run", str(REPO / "tools" / "mock-device.py"), "--port", str(port), "--quiet", *args],
            stdout=f, stderr=subprocess.STDOUT)
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
    const m = (await import('./js/web.js')).default;  // the app's instance (js/wasm.js)
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
    const m = (await import('./js/web.js')).default;  // the app's instance (js/wasm.js)
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
        goto_view(page, "settings/look")
        # Saved as chosen (no Save button).
        page.select_option("#st-trans", kind)
        page.select_option("#st-trans-ms", "400")
        cfg = wait_config(base, lambda c: c["transition"] == kind and c["transition_ms"] == 400)
        check(cfg["transition"] == kind and cfg["transition_ms"] == 400, f"settings: {label} / Normal saved ({cfg['transition']}:{cfg['transition_ms']})")
        goto_view(page, "now")
        r = mid_transition(page, base)
        check(r["vs_before"] > 0.1 * r["before_first"] and r["vs_first"] > 0.1 * r["before_first"],
              f"now: {label} 400 ms, {r['dt']:.0f} ms after the new file loaded the preview differs from the old "
              f"picture ({r['vs_before']} px) and from the new file's first frame ({r['vs_first']} px); "
              f"those two differ by {r['before_first']} px ({r['path']})")
    # The card's Preview: the panel's file, restarted with a transition, on
    # the page only (choosing Blinds saves it; the panel is not restarted).
    goto_view(page, "settings/look")
    page.select_option("#st-trans", "blinds")
    wait_config(base, lambda c: c["transition"] == "blinds")
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
    check(get_json(base + "/api/config")["transition"] == "blinds" and after["now_playing"]["ordinal"] == before["ordinal"],
          "settings: Blinds saved as chosen; Preview plays on the page only (the panel untouched)")
    page.select_option("#st-trans", "dissolve")
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
# (`?new=1`: a create-only upload whose name was taken answers 409 and the
# web app tries the next number: expected, the add test checks it.)
expected: list[str] = ["/cache/", "/sets/.smooth", "new=1"]


def unexpected(url: str) -> bool:
    return not any(e in url for e in expected)


def retried(url: str) -> bool:
    """A 503 here is the stream queue being full, and the web app asks again
    (thumbs.js for GIFs, source.js `queued` for listings and card reads)."""
    return any(p in url for p in ("/gifs/", "/api/files", "/api/cache/list", "/api/cache/failed"))


def new_page(browser, size: str, errors: list[str]) -> Page:
    w, h = SIZES[size]
    phone = size == "phone"
    ctx = browser.new_context(viewport={"width": w, "height": h}, device_scale_factor=3 if phone else 1,
                              is_mobile=phone, has_touch=phone)
    site_art_index(ctx)
    page = ctx.new_page()
    # (A CORS refusal's line names the URL in its text, not its location.)
    page.on("console", lambda m: m.type == "error" and unexpected(m.location.get("url", "")) and unexpected(m.text)
            and not ("status of 503" in m.text and retried(m.location.get("url", "")))
            and not ("status of 409" in m.text and "/api/files/" in m.location.get("url", "") and "new=1" in m.location.get("url", ""))
            and errors.append(f"[{size}] console: {m.text}"))
    page.on("pageerror", lambda e: errors.append(f"[{size}] pageerror: {e}"))
    # Chromium's console line for a failed load has no URL; this does.
    page.on("response", lambda r: r.status >= 400 and unexpected(r.url) and not (r.status == 503 and retried(r.url))
            and errors.append(f"[{size}] HTTP {r.status} {r.request.method} {r.url}"))
    # And which load a connection closed with no response was (Chromium's
    # ERR_EMPTY_RESPONSE console line has no URL either).
    page.on("requestfailed", lambda r: "EMPTY_RESPONSE" in (r.failure or "")
            and print(f"  ..   [{size}] no response: {r.method} {r.url}"))
    page.on("dialog", lambda d: d.accept())
    return page


def site_art_index(ctx) -> None:
    """The site's art-index/v1/ for this context, from opendmd-web/art-index/,
    with the site's `Access-Control-Allow-Origin: *`."""
    def answer(route) -> None:
        site_art_asked.append((route.request.url, route.request.headers.get("origin")))
        f = ART_INDEX / urllib.parse.urlsplit(route.request.url).path.rsplit("/", 1)[-1]
        if f.is_file():
            route.fulfill(status=200, body=f.read_bytes(), headers={"access-control-allow-origin": "*", "content-type": "application/json"})
        else:
            route.fulfill(status=404, body="", headers={"access-control-allow-origin": "*"})
    ctx.route(SITE_ART + "*", answer)


TREE = "document.querySelector('#lib-list')?._tree"


def pick_cat(page: Page, value: str) -> None:
    """Library: the folder `value` (a directory under gifs/, `@` Live) open at the top, the others closed: `#library/<folder>`, what a
    link to a folder does. A search or a platform chip would filter it: they
    are cleared first."""
    page.evaluate("location.hash = '#library'")
    page.wait_for_selector("#view-library.on")
    page.wait_for_selector("#lib-search", timeout=30000)
    if page.input_value("#lib-search"):
        page.fill("#lib-search", "")
    for chip in page.query_selector_all("#lib-chips .chip.on"):  # platforms picked without a search
        chip.click()
    page.evaluate(f"location.hash = '#library/' + encodeURIComponent({json.dumps(value)})")
    page.wait_for_function(f"(t => t && t.open.size === 1 && t.open.has({json.dumps(value)}) && t.rows.some(r => r.cat === {json.dumps(value)} && !r.path)"
                           f" && !t.rows.some(r => r.ld))({TREE})", timeout=30000)


def settings_tab_checks(page: Page) -> None:
    """Settings' four tabs: bare #settings is Playback, each tab its own
    address, only the open one in the page, a click changes the address."""
    goto_view(page, "settings")
    tabs = page.eval_on_selector_all("#st-tabs .tab", "els => els.map(e => [e.dataset.tab, e.textContent, e.classList.contains('on')])")
    check(tabs == [["play", "Playback", True], ["look", "Look", False], ["time", "Schedule", False], ["device", "Device", False]],
          f"settings: four tabs, bare #settings opens Playback ({tabs})")
    for t, sel in (("look", "#st-glow"), ("time", "#tm-zone"), ("device", "#st-hostname"), ("play", "#st-set")):
        page.click(f"#st-tabs [data-tab={t}]")
        page.wait_for_selector(f"#st-tab-{t}", timeout=20000)
        page.wait_for_selector(sel, timeout=20000)
        others = page.locator(".stab").count()
        check(page.evaluate("location.hash") == f"#settings/{t}" and others == 1,
              f"settings: the {t} tab at #settings/{t}, alone in the page ({others} tab panel(s))")
    check(page.locator("#st-adv").count() == 0 and page.locator("#st-plats .chip").count() == 0,
          "settings: no Advanced, no platform chip wall")


def goto_view(page: Page, view: str) -> None:
    """`view`, or `settings/<tab>` (play, look, time, device: only the open
    tab is in the page) or `sets/edit`."""
    page.evaluate(f"location.hash = '#{view}'")
    name, _, sub = view.partition("/")
    page.wait_for_selector(f"#view-{name}.on")
    if name == "settings":
        page.wait_for_selector(f"#st-tab-{sub or 'play'}", timeout=20000)
    page.wait_for_timeout(300)


# The headings' pixel font (www/fonts/silkscreen.woff2, tools/font-subset.py):
# per view, the selectors that must be in Silkscreen and those that must not.
FONT_VIEWS = {
    "now": (["#top strong", "#view-now .look>summary"], ["#now-skip", "#nav a", ".nowt b"]),
    "library": ([], [".lib-q input", ".vrow:not([hidden])", "#nav a"]),
    "sets": (["#view-sets .sl h2"], ["#sets-new", "#sets-list button.pl"]),
    "create": (["#view-create .st-box>h2", "#view-create section.card>h2"], ["#view-create .st-box>p"]),
    "settings": (["#view-settings .ptitle", "#view-settings .stab h2"], ["#st-tabs .tab", "#view-settings label"]),
    "settings/time": (["#view-settings .stab>.grp>h3"], ["#view-settings .stab .tick"]),
    "settings/device": (["#view-settings .stab>.grp>h3"], ["#view-settings .facts dd"]),
}


def font_checks(page: Page, base: str, size: str) -> None:
    """Silkscreen loads (self-hosted, `font/woff2`) and the headings use it at
    a multiple of its 8 px grid, weight 400, no letter-spacing; body text,
    buttons, inputs, tabs, nav and content titles do not. Each view is shot
    into OUT/font/ for a look at crispness and wrapping."""
    r = page.request.get(base + "/fonts/silkscreen.woff2")
    check(r.ok and r.headers.get("content-type", "").startswith("font/woff2") and r.body()[:4] == b"wOF2",
          f"font [{size}]: fonts/silkscreen.woff2 served as {r.headers.get('content-type')} ({len(r.body())} B)")
    page.evaluate("document.fonts.ready")
    loaded = page.evaluate("[...document.fonts].filter(f => f.family.replace(/\"/g, '') === 'Silkscreen').map(f => f.status)")
    check(loaded == ["loaded"], f"font [{size}]: the Silkscreen face loaded ({loaded})")
    (OUT / "font").mkdir(parents=True, exist_ok=True)
    js = """sels => sels.flatMap(s => [...document.querySelectorAll(s)].filter(e => e.offsetParent).map(e => {
        const c = getComputedStyle(e);
        return [s, e.textContent.trim().slice(0, 24), c.fontFamily.split(',')[0].replace(/"/g, ''), c.fontSize, c.fontWeight, c.letterSpacing,
                e.scrollWidth - e.clientWidth];
      }))"""
    for view, (pixel, plain) in FONT_VIEWS.items():
        goto_view(page, view)
        page.wait_for_timeout(500)
        heads = page.evaluate(js, pixel)
        bad = [h for h in heads if h[2] != "Silkscreen" or h[3] not in ("16px", "24px", "32px") or h[4] != "400"
               or h[5] not in ("normal", "0px") or h[6] > 0]
        check(bool(heads) or not pixel, f"font [{size}]: {view} has its headings ({len(heads)})")
        check(not bad, f"font [{size}]: {view} headings in Silkscreen at 16/24 px, 400, no letter-spacing, no overflow"
              + (f" ({bad})" if bad else ""))
        others = [h for h in page.evaluate(js, plain) if h[2] == "Silkscreen"]
        check(not others, f"font [{size}]: {view} body text, buttons, inputs and nav not in Silkscreen" + (f" ({others})" if others else ""))
        page.screenshot(path=str(OUT / "font" / f"{view.replace('/', '-')}-{size}.png"))
    # Content titles (computed even while the Details panel is closed).
    goto_view(page, "library")
    fam = page.eval_on_selector("#lib-d-title", "e => getComputedStyle(e).fontFamily")
    check("Silkscreen" not in fam, f"font [{size}]: a GIF's title (Details) stays in the body face ({fam[:30]})")
    goto_view(page, "now")


def wait_schedule(base: str, ok, timeout_s: float = 10) -> dict:
    deadline = time.time() + timeout_s
    while True:
        sched = get_json(base + "/api/schedule")
        if ok(sched) or time.time() > deadline:
            return sched
        time.sleep(0.2)


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


def toast_or_none(page: Page, kind: str, timeout: float = 10000) -> str:
    """The last toast of `kind`, or "" when none comes (a check then fails, not the run)."""
    try:
        page.wait_for_selector(f"#toasts .toast.{kind}", timeout=timeout)
    except PlaywrightTimeoutError:
        return ""
    return page.eval_on_selector_all(f"#toasts .toast.{kind}", "els => els.at(-1).textContent")


def send_from_library(page: Page, base: str, what: str) -> None:
    page.wait_for_selector("#lib-play:not([disabled])", timeout=10000)
    path = page.get_attribute("#lib-play", "data-path")
    page.click("#lib-play")
    cur = wait_playing(base, path)
    check(cur == path, f"{what}: Send to panel -> /api/status plays {path} (got {cur})")


def root_manifest_rows(page: Page) -> list[str]:
    return page.eval_on_selector_all("#sets-list [data-path^='/filelist']", "els => els.map(e => e.dataset.path)")


def row_label(page: Page, path: str) -> str:
    return page.eval_on_selector(f"#sets-list [data-path='{path}'] .grow", "e => e.firstChild.textContent")


def sets_do(page: Page, path: str, what: str) -> None:
    """Playlists: a row's "…" menu, then Activate or Delete."""
    page.click(f"#sets-list [data-path='{path}'] button.more")
    page.click(f"#sets-menu [data-do={what}]")


def add_files(page: Page, files) -> None:
    """Library: + Add GIFs and the files picked."""
    with page.expect_file_chooser() as fc:
        page.click("#lib-add-btn")
    fc.value.set_files([str(f) for f in files])


def wait_thumbs_idle(page: Page, timeout: int = 60000) -> None:
    """No thumbnail read in flight and every canvas in the view drawn (or
    given up on, in the Library's list): what is left on the card is not asked for again."""
    page.wait_for_function("""async () => {
        const { stats } = await import('./js/thumbs.js');
        return stats.busy === 0 && [...document.querySelectorAll('#lib-list canvas.th')].every((c) => c.parentNode.hidden || c.dataset.done);
    }""", timeout=timeout)


def details_do(page: Page, item: str) -> None:
    """Library Details: the "…" menu, then one of its items."""
    page.click("#lib-d-more")
    page.click(item)


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


def facets(page: Page, sel: str, attr: str) -> list:
    return page.eval_on_selector_all(f"{sel} .chip", f"els => els.map(e => [e.dataset.{attr}, +e.querySelector('small').textContent.replace(/,/g, ''), e.classList.contains('on')])")


def lib_count(page: Page) -> tuple[int, int]:
    a, b = page.inner_text("#lib-count").replace(",", "").split(" of ")
    return int(a), int(b)


def folder_counts(page: Page) -> list:
    """The folders the tree shows, with their counts: [[folder, n], ...]."""
    return page.evaluate(f"{TREE}.rows.filter(r => !r.path && !r.ld).map(r => [r.cat, r.n])")


def wait_counted(page: Page) -> None:
    page.wait_for_function("/^[\\d,]+ of [\\d,]+/.test(document.querySelector('#lib-count').textContent)", timeout=60000)


def search_checks(page: Page, exact: bool) -> None:
    """The Library's search: every GIF on the card whatever was open, the
    folders with matches open with their counts (the category facets the
    tabs used to become), platform chips with counts (most first) that
    narrow the tree, and the folder back as it was when the box is cleared.
    Leaves 'mario' with SNES on (open_library goes on from there). `exact`:
    the counts against the card copy (the mock's)."""
    gifs = [p for p in (CARD / "gifs").glob("*/*") if p.suffix.lower() == ".gif"]
    want = [p for p in gifs if "mario" in p.name.lower()]
    by_cat = {}
    for p in want:
        by_cat[p.parent.name] = by_cat.get(p.parent.name, 0) + 1
    tab_n = lib_count(page)[1]
    page.fill("#lib-search", "mario")
    wait_counted(page)
    shown, total = lib_count(page)
    cats = folder_counts(page)
    if not exact:
        want, gifs, by_cat = [None] * shown, [None] * total, {c: n for c, n in cats}
    check(total == len(gifs) and shown == len(want) == sum(by_cat.values()), f"library search: 'mario' over every GIF -> {shown} of {total} (want {len(want)} of {len(gifs)})")
    check({c: n for c, n in cats} == by_cat and all(n > 0 for _, n in cats),
          f"library search: only the folders with matches, each with its count ({cats[:4]}…)")
    opened = page.evaluate(f"{TREE}.rows.filter(r => !r.path && !r.ld).every(r => r.open)")
    check(opened, "library search: the folders with matches open")
    shot(page, "library-search", "laptop")
    plats = facets(page, "#lib-chips", "platform")
    snes = next((n for p, n, _ in plats if p == "SNES"), 0)
    check(sum(n for _, n, _ in plats) <= shown and snes > 0 and [n for _, n, _ in plats] == sorted((n for _, n, _ in plats), reverse=True),
          f"library search: platform chips, most first ({plats[:3]}…)")
    page.click("#lib-chips .chip[data-platform=SNES]")
    shown = lib_count(page)[0]
    cats = folder_counts(page)
    check(shown == snes and sum(n for _, n in cats) == snes, f"library search: + SNES -> {shown}, the folders counting SNES only ({cats[:3]}…)")
    # Instant on ~11k GIFs: five keystrokes, each a full pass and its chips.
    ms = page.evaluate("""async () => { const s = document.querySelector('#lib-search'), t = performance.now();
        for (const q of ['m', 'ma', 'mar', 'mari', 'mario']) { s.value = q; s.dispatchEvent(new Event('input')); await new Promise((r) => setTimeout(r)); }
        return (performance.now() - t) / 5; }""")
    check(ms < 50 and lib_count(page)[0] == snes, f"library search: {ms:.1f} ms a keystroke over {total:,} GIFs")
    # A folder closed in a search stays closed for it; its count stays.
    first = cats[0][0]
    page.click(f"#lib-list .vdir[data-cat={json.dumps(first)}]")
    page.wait_for_function(f"{TREE}.rows.some(r => r.cat === {json.dumps(first)} && !r.path && !r.open)", timeout=5000)
    left = snes - cats[0][1]
    check(lib_count(page)[0] == left and page.evaluate(f"{TREE}.res.every(it => !it.path.startsWith('/gifs/' + {json.dumps(first)} + '/'))"),
          f"library search: closing {first} takes its matches out of what is shown (and Select all shown) ({page.inner_text('#lib-count')})")
    page.fill("#lib-search", "")
    wait_counted(page)
    check(lib_count(page) == (tab_n, tab_n) and page.locator("#lib-chips .chip.on").count() == 0,
          f"library search: cleared -> the folder as it was, platforms off ({page.inner_text('#lib-count')})")
    page.fill("#lib-search", "mario")
    wait_counted(page)
    page.click("#lib-chips .chip[data-platform=SNES]")


def open_library(page: Page, exact: bool = True) -> None:
    goto_view(page, "library")
    pick_cat(page, "Consoles")
    wait_counted(page)
    search_checks(page, exact)
    count = page.inner_text("#lib-count")
    shown = int(count.split(" of ")[0].replace(",", ""))
    check(shown > 0, f"library: Consoles / 'mario' / SNES -> {count}")
    names = page.eval_on_selector_all("#lib-list .vrow:not([hidden]) .t", "els => els.map(e => e.textContent)")
    check(all("mario" in n.lower() for n in names[:shown]), f"library: rows are Mario titles ({names[:3]})")
    page.click("#lib-list .vrow:not([hidden]) >> nth=0")
    wait_lit(page, "#lib-panel", "library")


def draft(page: Page) -> list[str]:
    return page.evaluate("async () => { const { app } = await import('./js/state.js'); return [...app.draft.paths]; }")


def reorder_checks(page: Page, two: list[str]) -> None:
    """The playlist editor: Alt+↓ on an entry's handle moves it (the focus
    follows), Alt+↑ back; a drag of the handle moves it; the Arrange menu."""
    grip = "#set-entries .vrow:not([hidden]) >> nth=0 >> button.grip"
    page.focus(grip)
    page.keyboard.press("Alt+ArrowDown")
    focus = page.evaluate("document.activeElement.closest('.vrow')?._i")
    check(draft(page) == two[::-1] and focus == 1, f"playlists: Alt+Down moves the entry, focus follows ({draft(page)}, row {focus})")
    page.keyboard.press("Alt+ArrowUp")
    check(draft(page) == two, "playlists: Alt+Up moves it back")
    box = page.locator(grip).bounding_box()
    x, y = box["x"] + box["width"] / 2, box["y"] + box["height"] / 2
    page.mouse.move(x, y)
    page.mouse.down()
    page.mouse.move(x, y + 25, steps=4)
    page.mouse.move(x, y + 45, steps=4)
    page.mouse.up()
    check(draft(page) == two[::-1], f"playlists: dragging the handle down a row moves the entry ({draft(page)})")
    page.click("#set-arrange")
    items = page.eval_on_selector_all("#set-arrange-menu .mi", "els => els.map(e => e.textContent + (e.classList.contains('danger') ? '!' : ''))")
    check(items == ["Shuffle", "Sort A–Z", "Picked to top", "Remove picked!", "Clear!"], f"playlists: the Arrange menu ({items})")
    page.click("#set-sort")
    check(page.locator("#set-arrange-menu").count() == 0 and len(draft(page)) == 2, "playlists: Sort A–Z from Arrange")
    page.evaluate(f"async () => {{ const {{ app }} = await import('./js/state.js'); const {{ emit }} = await import('./js/util.js'); app.draft.paths = {json.dumps(two)}; emit('draft'); }}")


# The Library's rows in view (not the pooled rows above and below it).
IN_VIEW = ("[...document.querySelectorAll('#lib-list .vrow:not([hidden])')].filter(r => { const t = r.getBoundingClientRect().top,"
           " l = document.querySelector('#lib-list').getBoundingClientRect(); return t >= l.top && t < l.bottom - 40; })")


def card_requests(page: Page) -> dict:
    """The page's requests for card files (/gifs/, /cache/): how many are in
    flight now and at most, and every URL."""
    io = {"n": 0, "max": 0, "urls": []}
    card = lambda r: "/cache/" in r.url or "/gifs/" in r.url

    def start(r):
        if card(r):
            io["n"] += 1
            io["max"] = max(io["max"], io["n"])
            io["urls"].append(urllib.parse.unquote(urllib.parse.urlsplit(r.url).path))

    def end(r):
        if card(r):
            io["n"] -= 1
    page.on("request", start)
    page.on("requestfinished", end)
    page.on("requestfailed", end)
    return io


def thumbs_stats(page: Page) -> dict:
    return page.evaluate("import('./js/thumbs.js').then(m => ({ ...m.stats }))")


def thumbs_checks(browser, base: str, errors: list[str]) -> None:
    """Thumbnails (js/thumbs.js), on by default, in a fresh browser (an
    empty thumbnail store) with the mock pacing card files at 400 KB/s in
    the firmware's 2 KB pieces: the rows in view of Consoles get a first
    frame; at most two requests in flight (the page's and the mock's
    count); each read cut off after the first image (bytes per request);
    the prepared copy, never the GIF when there is one; a fling fetches only
    where it stops; a reload fetches nothing (IndexedDB); a hover plays a
    short clip, also kept; the switch in Settings turns them off."""
    page = new_page(browser, "laptop", errors)
    io = card_requests(page)
    page.goto(base + "/#settings/device")
    page.wait_for_selector("#st-thumbs", timeout=20000)
    check(page.is_checked("#st-thumbs"), "thumbnails: on by default")
    get_json(base + "/mock/io?reset=1&kbps=400")
    pick_cat(page, "Consoles")
    t0 = time.time()
    page.wait_for_function(f"{IN_VIEW}.length >= 10 && {IN_VIEW}.every(r => r.querySelector('canvas.th[data-done]'))", timeout=20000)
    rows = page.evaluate(f"{IN_VIEW}.map(r => r.dataset.path)")
    page.wait_for_timeout(500)
    m = get_json(base + "/mock/io")
    whole = sum((CARD / cache_path(p).lstrip("/")).stat().st_size for p in rows if (CARD / cache_path(p).lstrip("/")).exists())
    per = m["bytes"] / max(1, m["gets"])
    check(True, f"thumbnails: {len(rows)} rows in view drawn in {time.time() - t0:.1f} s: {m['gets']} requests, {m['bytes']:,} B sent"
          f" ({per:,.0f} B a request; their whole copies are {whole:,} B), {m['cut']} cut off")
    check(m["gets"] <= len(rows) + 2, f"thumbnails: only the rows in view asked ({m['gets']} requests for {len(rows)} rows)")
    check(io["max"] <= 2 and m["max"] <= 2, f"thumbnails: at most 2 requests in flight (page {io['max']}, mock {m['max']})")
    # (The reader hangs up after ~12 KB; the mock has sent a piece or two more by
    # the time it notices, more on a loaded machine: 16 KB, still far from whole.)
    check(per < 16 * 1024 and m["bytes"] < whole / 4, f"thumbnails: each read stops after the first frame ({per:,.0f} B a request)")
    lit = page.evaluate("""() => Math.max(0, ...[...document.querySelectorAll('#lib-list .vrow:not([hidden]) canvas.th[data-done="1"]')].map((c) => {
        const d = c.getContext('2d').getImageData(0, 0, 128, 32).data; let n = 0;
        for (let i = 0; i < d.length; i += 4) n += d[i] + d[i + 1] + d[i + 2] > 60; return n; }))""")
    check(lit > 50, f"thumbnails: a first frame drawn ({lit} lit px)")
    copied = [u for u in io["urls"] if u.startswith("/gifs/") and (CARD / cache_path(u).lstrip("/")).exists()]
    check(not copied, f"thumbnails: the prepared copy, never the GIF when there is one ({copied[:2]})")
    shot(page, "library-thumbs", "laptop")
    # A fling: nothing on the way, the screen where it stops.
    get_json(base + "/mock/io?reset=1")
    page.evaluate("""async () => { const l = document.querySelector('#lib-list');
        for (let i = 0; i < 30; i++) { l.scrollTop += 4000; await new Promise((r) => setTimeout(r, 12)); } }""")
    page.wait_for_function(f"{IN_VIEW}.length >= 10 && {IN_VIEW}.every(r => r.querySelector('canvas.th[data-done]'))", timeout=20000)
    page.wait_for_timeout(500)
    m = get_json(base + "/mock/io")
    check(m["gets"] <= len(rows) + 2, f"thumbnails: a fling through {30 * 4000 // 44:,} rows fetches only where it stops ({m['gets']} requests)")
    page.eval_on_selector("#lib-list", "e => { e.scrollTop = 0; }")
    # A reload: every one of them from the browser's store.
    page.reload()
    get_json(base + "/mock/io?reset=1")
    io["urls"].clear()
    pick_cat(page, "Consoles")
    page.wait_for_function(f"{IN_VIEW}.length >= 10 && {IN_VIEW}.every(r => r.querySelector('canvas.th[data-done]'))", timeout=20000)
    page.wait_for_timeout(500)
    m, st = get_json(base + "/mock/io"), thumbs_stats(page)
    check(m["gets"] == 0 and st["idb"] >= len(rows) - 1, f"thumbnails: after a reload none fetched again ({m['gets']} requests, {st['idb']} from the store)")
    # Hover: a short clip over the row, read once.
    # (A GIF that starts dark had its clip read with its still: a row without one.)
    bare = page.evaluate("""async (paths) => {
        const d = (await indexedDB.databases()).find((x) => x.name.startsWith('odmd-thumbs-'));
        const db = await new Promise((r) => { const q = indexedDB.open(d.name); q.onsuccess = () => r(q.result); });
        const out = [];
        for (const p of paths) {
            const rec = await new Promise((r) => { const q = db.transaction('t').objectStore('t').get(p); q.onsuccess = () => r(q.result); });
            if (rec && rec.still && !rec.clip) out.push(p);
        }
        db.close();
        return out; }""", rows)
    get_json(base + "/mock/io?reset=1")
    row = f"#lib-list .vrow[data-path={json.dumps(bare[0])}]"
    page.hover(f"{row} .t")
    page.wait_for_selector(f"{row} img.th", timeout=10000)
    m = get_json(base + "/mock/io")
    check(m["gets"] in (1, 2) and m["bytes"] <= 40 * 1024, f"thumbnails: a hover plays a clip ({m['gets']} request, {m['bytes']:,} B; {len(rows) - len(bare)} of {len(rows)} rows started dark and read theirs with the still)")
    page.mouse.move(5, 5)
    page.wait_for_selector(f"{row} img.th", state="detached", timeout=5000)
    page.reload()
    get_json(base + "/mock/io?reset=1")
    pick_cat(page, "Consoles")
    page.wait_for_selector(f"{row} canvas.th[data-done='1']", timeout=20000)
    # (Once the page has settled: a scroll of the list, as its rows come, ends a hover.)
    page.wait_for_timeout(1500)
    page.hover(f"{row} .t")
    page.wait_for_selector(f"{row} img.th", timeout=10000)
    check(get_json(base + "/mock/io")["gets"] == 0, "thumbnails: the clip again after a reload, from the store")
    load_checks(page, base)
    get_json(base + "/mock/io?kbps=0")
    goto_view(page, "settings/device")
    page.uncheck("#st-thumbs")
    goto_view(page, "library")
    check(page.locator("#lib-list .vrow:not([hidden]) canvas.th").count() == 0, "thumbnails: off -> no canvases")
    page.context.close()


def card_swap_checks(browser, base: str, errors: list[str]) -> None:
    """Card identity (`sd.card_id`): what the browser keeps is per card.
    With Consoles' thumbnails drawn, the mock swaps the card (`GET
    /mock/card?id=`: a power cycle with another card in, the same paths);
    the page notices on its next status (here a return to the tab), lists
    the folders again and fetches the thumbnails afresh into the new card's
    own store instead of showing the first card's; the folder counts are
    kept per card too. Swapped back, a reload draws them all from the first
    card's store."""
    page = new_page(browser, "laptop", errors)
    first = get_json(base + "/mock/card")["card_id"]
    other = "00000000000000b2" if first != "00000000000000b2" else "00000000000000c3"
    check(get_json(base + "/api/status")["sd"]["card_id"] == first, f"card id: in /api/status sd ({first})")
    get_json(base + "/mock/io?reset=1&kbps=0")
    page.goto(base + "/#library")
    pick_cat(page, "Consoles")
    rows = drawn(page)
    page.wait_for_timeout(500)
    stores = lambda: page.evaluate("indexedDB.databases().then((l) => l.map((d) => d.name).filter((n) => n.startsWith('odmd-thumbs-')).sort())")
    check(stores() == [f"odmd-thumbs-i{first}"], f"card swap: the thumbnails stored under the card's id ({stores()})")
    try:
        get_json(base + f"/mock/card?id={other}")
        get_json(base + "/mock/io?reset=1")
        # What must not happen is the first card's thumbnail drawn once the
        # page knows of the swap. Counting the thumbnails store reads
        # (`stats.idb`) is not that: after the swap the new card's own store
        # is read too (a row drawn again while its thumbnail is being stored;
        # a read made while the store's write is open sees it), and on a loaded
        # machine that overlap is common. So follow each decoded image to its
        # source: a record read from the first card's store, or any image
        # made before the card event, drawn after it, is a stale thumbnail.
        page.evaluate("""async (first) => {
            const { on } = await import('./js/util.js');
            window.__cards = 0; window.__stale = [];
            const t = await import('./js/thumbs.js'); let idb0 = null; window.__idbSince = () => t.stats.idb - idb0;
            let armed = false; const made = new WeakMap();
            const get = IDBObjectStore.prototype.get;
            IDBObjectStore.prototype.get = function (k) {
                const r = get.call(this, k), db = this.transaction.db.name;
                r.addEventListener('success', () => { const s = r.result?.still; if (s && typeof s == 'object') try { s.__db = db; } catch {} });
                return r;
            };
            const cib = window.createImageBitmap;
            window.createImageBitmap = function (src, ...a) {
                const was = armed;
                return cib.call(this, src, ...a).then((bm) => { made.set(bm, { db: src?.__db, was }); return bm; });
            };
            const draw = CanvasRenderingContext2D.prototype.drawImage;
            CanvasRenderingContext2D.prototype.drawImage = function (img, ...a) {
                const m = armed && made.get(img);
                if (m) {
                    if (!m.was || m.db == 'odmd-thumbs-i' + first) window.__stale.push(m);
                }
                return draw.call(this, img, ...a);
            };
            on('card', () => { window.__cards++; armed = true; idb0 = t.stats.idb; });
        }""", first)
        page.evaluate("document.dispatchEvent(new Event('visibilitychange'))")
        deadline = time.time() + 20
        while time.time() < deadline and get_json(base + "/mock/io")["gets"] < len(rows) - 2:
            page.wait_for_timeout(200)
        now = drawn(page)
        page.wait_for_timeout(500)
        m = get_json(base + "/mock/io")
        cards, stale, idb = page.evaluate("[window.__cards, window.__stale, window.__idbSince?.()]")
        check(m["gets"] >= len(now) - 2 and cards == 1 and not stale,
              f"card swap: another card's thumbnails fetched afresh, none of the first card's shown ({m['gets']} requests for {len(now)} rows, "
              f"{len(stale)} stale images drawn after the card event, {idb} reads of the new card's own store after it, {cards} card events)")
        check(stores() == sorted([f"odmd-thumbs-i{first}", f"odmd-thumbs-i{other}"]), f"card swap: one store per card ({stores()})")
        keys = page.evaluate("Object.keys(localStorage).filter((k) => k.startsWith('odmd.lib.n')).sort()")
        check(keys == sorted([f"odmd.lib.n.i{first}", f"odmd.lib.n.i{other}"]), f"card swap: folder counts kept per card ({keys})")
        # Replies out of order: a status asked before the swap and answered
        # after it (the Now poll, a return to the tab, a view's refresh are
        # each a request) carries the first card's id. It is its caller's;
        # the app's status, and the card it is on, stay the newer one's.
        flips = page.evaluate("""async (first) => {
            const { app } = await import('./js/state.js'); const { on } = await import('./js/util.js');
            let cards = 0; on('card', () => cards++);
            const real = app.src.status.bind(app.src), late = JSON.parse(JSON.stringify(app.status));
            late.sd.card_id = first;
            let n = 0;
            app.src.status = (r) => n++ ? real(r) : new Promise((res) => setTimeout(() => res(late), 600));
            try { await Promise.all([app.refreshStatus(), app.refreshStatus()]); } finally { app.src.status = real; }
            await new Promise((r) => setTimeout(r, 200));
            return [cards, app.cardKey];
        }""", first)
        check(flips == [0, f"i{other}"], f"card swap: a status reply landing after a newer one does not bring the old card back ({flips})")
    finally:
        get_json(base + f"/mock/card?id={first}")
    # A store named by the card's size (firmware before `card_id`) goes.
    page.evaluate("new Promise((r) => { const q = indexedDB.open('odmd-thumbs-c31266439168', 1); q.onsuccess = () => { q.result.close(); r(); }; })")
    page.reload()
    get_json(base + "/mock/io?reset=1")
    pick_cat(page, "Consoles")
    drawn(page)
    page.wait_for_timeout(500)
    m = get_json(base + "/mock/io")
    check(m["gets"] == 0, f"card swap: the first card back, its thumbnails all from its store ({m['gets']} requests)")
    check("odmd-thumbs-c31266439168" not in stores(), f"card swap: the store named by the card's size alone removed ({stores()})")
    page.context.close()


STORED = """async (paths) => {
    const d = (await indexedDB.databases()).find((x) => x.name.startsWith('odmd-thumbs-'));
    const db = await new Promise((r) => { const q = indexedDB.open(d.name); q.onsuccess = () => r(q.result); });
    const out = {};
    for (const p of paths) out[p] = await new Promise((r) => { const q = db.transaction('t').objectStore('t').get(p); q.onsuccess = () => r(q.result ? (q.result.still ? 'still' : 'none') : null); });
    db.close();
    return out; }"""


def drawn(page: Page, timeout: int = 30000) -> list[str]:
    page.wait_for_function(f"{IN_VIEW}.length >= 10 && {IN_VIEW}.every(r => r.querySelector('canvas.th[data-done]'))", timeout=timeout)
    return page.evaluate(f"{IN_VIEW}.map(r => r.dataset.path)")


def load_checks(page: Page, base: str) -> None:
    """The thumbnails under a device's stream task (the mock serves card
    files one at a time, two waiting, 503 + Retry-After 1 after 2 s, paced
    at 400 KB/s): a dropped request is not kept as a failure and is asked
    for again on a later visit; injected 503s back off and still draw; a
    screen load with Details open (the preview's whole file in the same
    queue), and a hover while scrolling, measured."""
    # Failures that pass (here 500s): blank for now, not kept, drawn on the next visit.
    expected.append("/gifs/Arcade/")
    get_json(base + "/mock/io?reset=1&fail500=3")
    pick_cat(page, "Arcade")
    rows = drawn(page)
    deadline = time.time() + 20
    while thumbs_stats(page)["failed"] < 1 and time.time() < deadline:
        page.wait_for_timeout(200)
    kept = page.evaluate(STORED, rows)
    blank = [p for p in rows if kept[p] is None]
    check(len(blank) >= 1 and not any(v == "none" for v in kept.values()),
          f"load: {len(blank)} rows whose request failed (500) are not kept as failures ({thumbs_stats(page)['failed']} failed)")
    page.reload()
    pick_cat(page, "Arcade")
    drawn(page)
    page.wait_for_timeout(800)
    kept = page.evaluate(STORED, blank)
    check(all(kept[p] == "still" for p in blank), f"load: a later visit asks for them again and draws them ({kept})")
    expected.remove("/gifs/Arcade/")
    # 503s: everything waits Retry-After (1 s), then the rows draw.
    get_json(base + "/mock/io?reset=1&fail503=2")
    t0 = time.time()
    pick_cat(page, "Computers")
    rows = drawn(page)
    page.wait_for_function(f"{IN_VIEW}.every(r => r.querySelector('canvas.th[data-done=\"1\"]'))", timeout=20000)
    st = thumbs_stats(page)
    check(st["r503"] >= 2 and time.time() - t0 >= 1, f"load: two 503s back off (r503 {st['r503']}) and the screen still draws ({time.time() - t0:.1f} s)")
    # A screen load with Details open: the preview's file in the same queue.
    get_json(base + "/mock/io?reset=1")
    lat: list[float] = []
    stop = threading.Event()

    def poll():
        while not stop.is_set():
            t = time.time()
            get_json(base + "/api/status")
            lat.append((time.time() - t) * 1000)
            stop.wait(0.5)
    th = threading.Thread(target=poll)
    th.start()
    r0 = thumbs_stats(page)["r503"]
    pick_cat(page, "Pinball_Short")
    page.click("#lib-list .vrow:not([hidden]) >> nth=0")
    t0 = time.time()
    drawn(page, 60000)
    page.wait_for_function("document.querySelector('#lib-cap').dataset.shown", timeout=60000)
    took = time.time() - t0
    stop.set()
    th.join()
    m, st = get_json(base + "/mock/io"), thumbs_stats(page)
    lat.sort()
    check(m["max"] <= 2, f"load: Pinball_Short with Details open: {m['gets']} card requests, {m['bytes']:,} B, {m['r503']} 503s (page r503 +{st['r503'] - r0}),"
          f" {took:.1f} s; /api/status p50 {lat[len(lat) // 2]:.0f} ms, max {lat[-1]:.0f} ms")
    # Hovering while scrolling: clips only where the pointer rests.
    get_json(base + "/mock/io?reset=1")
    box = page.locator("#lib-list").bounding_box()
    page.mouse.move(box["x"] + 200, box["y"] + 100)
    for _ in range(12):
        page.mouse.wheel(0, 120)
        page.wait_for_timeout(120)
    page.wait_for_timeout(1500)
    m = get_json(base + "/mock/io")
    rows = drawn(page)
    kept = page.evaluate(STORED, rows)
    check(not any(v == "none" for v in kept.values()) and m["max"] <= 2,
          f"load: hovering while scrolling: {m['gets']} requests, {m['bytes']:,} B, {m['r503']} 503s, none kept as failed")


def wait_mock_idle(base: str, timeout: float = 20.0) -> None:
    """No card file in flight in the mock (a closed page's reads end at their
    next write): the stream queue is free for what the next check expects."""
    deadline = time.time() + timeout
    while time.time() < deadline and get_json(base + "/mock/io")["inflight"] > 0:
        time.sleep(0.1)


def listing_queue_checks(browser, base: str, errors: list[str]) -> None:
    """Listings go out from the stream task (firmware http.rs `listing`, the
    mock likewise): with a file streaming and two waiting (the mock paced at
    5 KB/s), GET /api/files is a 503 with Retry-After: 1 at once (a listing
    does not wait for a place, as a file does), while /api/status answers;
    the web app asks again (source.js `queued`, every 2-4 s for ~45 s) and
    the folder opens once the queue drains, within that budget."""
    big = sorted((p for p in (CARD / "cache" / "Arcade").rglob("*.gif") if 40_000 < p.stat().st_size < 60_000))[:3]
    urls = [base + urllib.parse.quote("/" + p.relative_to(CARD).as_posix()) for p in big]
    expected.append("/api/files")
    page = new_page(browser, "laptop", errors)
    # No folder open, no thumbnails: only the listings meet the full queue.
    page.context.add_init_script("try { localStorage.setItem('odmd.lib.open', '[]'); localStorage.setItem('odmd.thumbs', 'false') } catch {}")
    # Nothing of an earlier page may still hold the stream queue: a listing
    # that met it would be a 503 here, asked again for ~45 s, then given up
    # (the tree keeps no folders and does not ask a third time).
    wait_mock_idle(base)
    get_json(base + "/mock/io?reset=1&kbps=0")
    seen503: list[str] = []
    page.on("response", lambda r: r.status == 503 and seen503.append(r.url))
    page.goto(base + "/#library")  # the Library's modules in before the queue fills
    try:
        page.wait_for_function(f"{TREE}?.folders.length > 2", timeout=60000)
    except PlaywrightTimeoutError:
        # A failure of its own (not a traceback that ends the run), with what
        # the page and the mock were doing.
        state = page.evaluate(f"({{ tree: !!{TREE}, folders: {TREE}?.folders.length, toast: document.querySelector('.toast, #toasts')?.textContent || '', mod: typeof customElements }})")
        check(False, f"listing queue: the Library lists the card's folders ({state}; {len(seen503)} 503s; mock {get_json(base + '/mock/io')})")
        expected.remove("/api/files")
        page.context.close()
        return
    page.evaluate("location.hash = '#sets'")
    page.wait_for_selector("#sets-list [data-path]", timeout=60000)
    page.wait_for_timeout(1000)
    get_json(base + "/mock/io?reset=1&kbps=5")

    def fetch(u):
        try:
            urllib.request.urlopen(u, timeout=60).read()
        except urllib.error.HTTPError:
            pass
    hold = [threading.Thread(target=fetch, args=(u,)) for u in urls]
    for t in hold:
        t.start()
        time.sleep(0.2)
    t0 = time.time()
    try:
        urllib.request.urlopen(base + "/api/files?dir=/gifs/Arcade", timeout=30).read()
        code, after = 200, None
    except urllib.error.HTTPError as e:
        code, after = e.code, e.headers.get("Retry-After")
    waited = time.time() - t0
    t1 = time.time()
    get_json(base + "/api/status")
    st_ms = (time.time() - t1) * 1000
    check(code == 503 and after == "1" and waited < 0.5 and st_ms < 300,
          f"listing queue: a full stream queue makes /api/files a 503 + Retry-After {after} at once ({waited:.2f} s); /api/status meanwhile {st_ms:.0f} ms")
    seen: dict[str, list[int]] = {}
    page.on("response", lambda r: "/api/files?dir=" in r.url and seen.setdefault(r.url, []).append(r.status))
    t0 = time.time()
    page.evaluate("location.hash = '#library/Pixel_Art'")
    page.wait_for_function(f"(t => t && t.rows.some(r => r.path?.startsWith('/gifs/Pixel_Art/')))({TREE})", timeout=60000)
    took = time.time() - t0
    worst = max((v.count(503) for v in seen.values()), default=0)
    # queued's budget: 45 s at 2 s or more a try.
    pix = next((v for u, v in seen.items() if "Pixel_Art" in u), [])
    check(worst >= 1 and worst <= 23 and pix[-1:] == [200] and took < 45,
          f"listing queue: the web app asks again and opens the folder in {took:.1f} s, at most {worst} 503s a listing (budget 23; {len(seen)} listings)")
    for t in hold:
        t.join()
    get_json(base + "/mock/io?kbps=0")
    expected.remove("/api/files")
    page.context.close()


def tree_checks(browser, base: str, errors: list[str]) -> None:
    """The Library's folder tree in a fresh browser: the folders (Live
    after the card's, system ones last), ⊟, the keys (↓ → → ↓ Enter
    ← ←), Consoles virtualised (3,781 GIFs, a few dozen rows in the page,
    the last GIF reached by End), a link to a folder, Select's folder tick;
    shots at laptop and phone widths."""
    for size in ("laptop", "phone"):
        page = new_page(browser, size, errors)
        page.goto(base + "/#library")
        page.wait_for_function(f"{TREE}?.folders.length > 2", timeout=30000)
        page.click("#lib-fold")
        page.wait_for_function(f"{TREE}.rows.every(r => !r.path)", timeout=5000)
        folders = page.evaluate(f"{TREE}.folders.map(f => [f.cat, f.name])")
        real = [f for f in folders if not f[0].startswith("_")]
        check(real[-1:] == [["@", "Live"]] and len(real) > 2 and not any(f[0] == "@fx" or f[1] == "Effects" for f in folders)
              and page.locator("#lib-cats, #lib-csel").count() == 0,
              f"tree [{size}]: the card's folders, then Live, system ones last; no Effects, no tabs, no select ({folders[-3:]})")
        if size == "phone":
            pick_cat(page, "Consoles")
            shot(page, "library-tree", "phone")
            page.context.close()
            continue
        ci = [f[0] for f in folders].index("Consoles")
        page.focus("#lib-list .vdir >> nth=0")
        for _ in range(ci):
            page.keyboard.press("ArrowDown")
        focus = page.evaluate("document.activeElement.dataset.cat")
        page.keyboard.press("ArrowRight")
        page.wait_for_function(f"{TREE}.rows.filter(r => r.path).length === 3781", timeout=30000)
        exp = page.evaluate("document.activeElement.getAttribute('aria-expanded')")
        check(focus == "Consoles" and exp == "true", f"tree: ↓ to Consoles ({focus}), → opens it (aria-expanded {exp})")
        page.keyboard.press("ArrowRight")
        page.keyboard.press("ArrowDown")
        first = page.evaluate("document.activeElement.dataset.path")
        page.keyboard.press("Enter")
        page.wait_for_function(f"(document.querySelector('#lib-cap').dataset.shown || '') === {json.dumps(first.rsplit('/', 1)[-1] if first else '')}", timeout=20000)
        check(first and first.startswith("/gifs/Consoles/"), f"tree: → steps in, ↓, Enter opens the details of {first}")
        page.keyboard.press("ArrowLeft")
        back = page.evaluate("document.activeElement.dataset.cat")
        page.keyboard.press("ArrowLeft")
        page.wait_for_function(f"{TREE}.rows.every(r => !r.path)", timeout=5000)
        check(back == "Consoles", f"tree: ← steps out to the folder ({back}), ← closes it")
        # Virtualised: the model has every GIF, the page a screenful.
        page.keyboard.press("ArrowRight")
        page.wait_for_function(f"{TREE}.rows.filter(r => r.path).length === 3781", timeout=30000)
        page.keyboard.press("End")
        last = page.evaluate(f"{TREE}.rows.at(-1)")
        page.wait_for_function("document.activeElement?.dataset?.cat || document.activeElement?.dataset?.path", timeout=5000)
        dom = page.locator("#lib-list .vrow, #lib-list .vdir").count()
        at = page.evaluate("document.activeElement.dataset.path || document.activeElement.dataset.cat")
        check(dom <= 40 and at == (last.get("path") or last.get("cat")), f"tree: Consoles' 3,781 GIFs in {dom} rows; End reaches the last row ({at})")
        shot(page, "library-tree", "laptop")
        # A link to a folder: it alone, open, at the top.
        page.goto(base + "/#library/" + urllib.parse.quote("NEW 2021"))
        page.wait_for_function(f"(t => t && t.open.size === 1 && t.open.has('NEW 2021') && t.rows.some(r => r.path))({TREE})", timeout=30000)
        top = page.evaluate("document.querySelector('#lib-list').scrollTop")
        idx = page.evaluate(f"{TREE}.rows.findIndex(r => r.cat === 'NEW 2021' && !r.path)")
        check(abs(top - idx * 44) < 2, f"tree: #library/NEW%202021 opens that folder at the top ({top} px, row {idx})")
        # Select: a folder's tick takes every GIF in it.
        page.click("#lib-select")
        page.click("#lib-list .vdir[data-cat='NEW 2021'] input[type=checkbox]")
        n = len(list((CARD / "gifs" / "NEW 2021").glob("*.gif")))
        page.wait_for_function(f"document.querySelector('#lib-selbar b').textContent === '{n}'", timeout=10000)
        check(page.is_checked("#lib-list .vdir[data-cat='NEW 2021'] input[type=checkbox]"), f"tree: Select's folder tick picks its {n} GIFs")
        page.click("#lib-sel-done")
        no_leaks(page, "library (tree)")
        page.context.close()


def entries(page: Page) -> list[str]:
    return page.evaluate("async () => { const { app } = await import('./js/state.js'); return [...app.draft.paths]; }")


def editor_checks(browser, base: str, overlay: Path, errors: list[str]) -> None:
    """Playlist editing by browsing (js/pledit.js), in a fresh browser: + New
    playlist, + Add GIFs (the folder tree beside the entries on a laptop):
    three GIFs ticked in Arcade and added in that order (✓ on them in the
    tree); a whole folder ticked (Pixel_Art); a search with a platform and
    "Add all N shown" (those already in skipped, said in the toast);
    thumbnails on the entries; Alt+↓ and ✕; saved, the file holding the
    entries in order. Then the phone: the panes take turns."""
    page = new_page(browser, "laptop", errors)
    page.goto(base + "/#sets")
    page.wait_for_selector("#sets-new", timeout=20000)
    page.click("#sets-new")
    page.wait_for_selector("#set-editor", state="visible", timeout=10000)
    check(page.is_hidden("#set-add-pane") and page.locator("#set-add-list .vdir").count() == 0,
          "editor: the Add pane (and its folder listing) only once asked for")
    page.click("#set-add")
    page.wait_for_selector("#set-add-pane", state="visible", timeout=5000)
    page.wait_for_selector("#set-add-list .vdir[data-cat=Arcade]", timeout=20000)
    check(page.is_visible("#set-ent-pane") and page.is_hidden("#sets-list"),
          "editor: on a laptop the tree sits beside the entries, the playlist list steps aside")
    page.click("#set-add-list .vdir[data-cat=Arcade]")
    page.wait_for_selector("#set-add-list .vrow:not([hidden])", timeout=20000)
    picks = [page.get_attribute(f"#set-add-list .vrow:not([hidden]) >> nth={i}", "data-path") for i in (2, 0, 1)]
    for p in picks:
        page.click(f"#set-add-list .vrow[data-path={json.dumps(p)}] .t")
    check(page.inner_text("#set-add-ticked") == "Add 3 ticked", f"editor: three ticked ({page.inner_text('#set-add-ticked')})")
    page.click("#set-add-ticked")
    page.wait_for_function("document.querySelector('#set-count').textContent.startsWith('3 ')", timeout=5000)
    marks = page.locator("#set-add-list .vrow:not([hidden]) i.inpl").count()
    check(entries(page) == picks and marks == 3 and page.inner_text("#set-add-ticked") == "Add 0 ticked",
          f"editor: added in the order ticked, ✓ on them in the tree ({marks})")
    # A whole folder.
    page.click("#set-add-list .vdir[data-cat=Arcade]")  # closed again
    page.wait_for_selector("#set-add-list .vdir[data-cat=Pixel_Art]", timeout=5000)
    page.click("#set-add-list .vdir[data-cat=Pixel_Art] input[type=checkbox]")
    pix = sorted(f"/gifs/Pixel_Art/{p.name}" for p in (CARD / "gifs" / "Pixel_Art").glob("*.gif"))
    page.wait_for_function(f"document.querySelector('#set-add-ticked').textContent === 'Add {len(pix)} ticked'", timeout=20000)
    page.click("#set-add-ticked")
    got = entries(page)
    check(sorted(got[3:]) == pix and len(got) == 3 + len(pix), f"editor: a folder's tick adds its {len(pix)} GIFs")
    # A search's results: every SNES Mario, from every folder.
    page.fill("#set-add-q", "mario")
    page.wait_for_selector("#set-add-chips .chip[data-platform=SNES]", timeout=60000)
    page.click("#set-add-chips .chip[data-platform=SNES]")
    page.wait_for_function("/^Add all [1-9]/.test(document.querySelector('#set-add-all').textContent)", timeout=20000)
    n = int(page.inner_text("#set-add-all").split()[2].replace(",", ""))
    want = sorted(f"/gifs/{p.parent.name}/{p.name}" for p in (CARD / "gifs").glob("*/*.gif")
                  if "mario" in p.name.lower() and re.match(r"^(?:(?:HALLOWEEN|XMAS)_)?SNES_", p.name, re.I))
    check(n == len(want), f"editor: 'mario' + SNES -> Add all {n} shown (the card has {len(want)})")
    folder = page.evaluate("document.querySelector('#set-add-list')._tree.rows.find(r => !r.path).cat")
    in_it = page.evaluate(f"document.querySelector('#set-add-list')._tree.rows.find(r => r.cat === {json.dumps(folder)} && !r.path).hits.length")
    page.click(f"#set-add-list .vdir[data-cat={json.dumps(folder)}]")
    page.wait_for_function(f"document.querySelector('#set-add-all').textContent === 'Add all {n - in_it} shown'", timeout=5000)
    check(True, f"editor: closing {folder} in the search leaves its {in_it} out of Add all ({n - in_it})")
    page.click(f"#set-add-list .vdir[data-cat={json.dumps(folder)}]")
    page.wait_for_function(f"document.querySelector('#set-add-all').textContent === 'Add all {n} shown'", timeout=5000)
    page.click("#set-add-all")
    before = len(got)
    got = entries(page)
    check(sorted(got[before:]) == want, f"editor: Add all adds the {len(want)} results")
    page.click("#set-add-all")
    t = last_toast(page, "ok")
    check(len(entries(page)) == len(got) and "0 added" in t and "already in it" in t, f"editor: adding them again adds none ('{t}')")
    # Thumbnails on the entries, in view only.
    in_view = IN_VIEW.replace("#lib-list", "#set-entries")
    page.wait_for_function(f"{in_view}.length >= 5 && {in_view}.every(r => r.querySelector('canvas.th[data-done]'))", timeout=20000)
    shot(page, "playlist-add", "laptop")
    page.click("#set-add-done")
    page.wait_for_selector("#set-add-pane", state="hidden", timeout=5000)
    # Reorder and remove.
    page.eval_on_selector("#set-entries", "e => { e.scrollTop = 0; }")
    page.wait_for_function("document.querySelector('#set-entries .vrow:not([hidden])')._i === 0", timeout=5000)
    page.focus("#set-entries .vrow:not([hidden]) >> nth=0 >> button.grip")
    page.keyboard.press("Alt+ArrowDown")
    order = entries(page)
    check(order[:2] == [got[1], got[0]], "editor: Alt+↓ moves the first entry down")
    page.hover("#set-entries .vrow:not([hidden]) >> nth=2")
    gone = order[2]
    page.click("#set-entries .vrow:not([hidden]) >> nth=2 >> button.rm")
    order = entries(page)
    check(gone not in order and len(order) == len(got) - 1, f"editor: ✕ removes {gone.rsplit('/', 1)[1]}")
    page.fill("#set-name", "browse")
    page.click("#set-save")
    page.wait_for_selector("#sets-list [data-path='/sets/browse.txt']", timeout=30000)
    f = overlay / "sets" / "browse.txt"
    check(f.exists() and f.read_text().split("\n")[:-1] == order, f"editor: saved, sets/browse.txt holds the {len(order)} entries in order")
    no_leaks(page, "playlists (add)")
    page.context.close()
    # The phone: one pane at a time.
    phone = new_page(browser, "phone", errors)
    phone.goto(base + "/#sets")
    phone.click("#sets-list [data-path='/sets/browse.txt'] button.pl")
    phone.wait_for_selector("#set-editor", state="visible", timeout=10000)
    phone.click("#set-add")
    phone.wait_for_selector("#set-add-list .vdir", timeout=20000)
    check(phone.is_hidden("#set-entries") and phone.is_visible("#set-add-list"), "editor [phone]: Add GIFs takes the entries' place")
    phone.click("#set-add-list .vdir[data-cat=Consoles]")
    phone.wait_for_selector("#set-add-list .vrow:not([hidden]) canvas.th[data-done]", timeout=20000)
    wide = phone.evaluate("document.documentElement.scrollWidth - document.documentElement.clientWidth")
    check(wide <= 0, f"editor [phone]: no sideways scroll ({wide}px)")
    shot(phone, "playlist-add", "phone")
    phone.click("#set-add-done")
    phone.wait_for_selector("#set-entries", state="visible", timeout=5000)
    check(phone.is_hidden("#set-add-pane"), "editor [phone]: Done brings the entries back")
    phone.context.close()
    urllib.request.urlopen(urllib.request.Request(base + "/api/files/sets/browse.txt", method="DELETE"), timeout=30)


PLAYING = "(document.querySelector('#sets-list .tag.act')?.closest('button.pl').textContent || '')"


def open_builder(page: Page) -> None:
    """Playlists: Add by filter, the sheet over the editor (a new playlist
    when none is open)."""
    if not page.is_visible("#set-build"):
        page.click("#sets-new")
    page.click("#set-build")
    page.wait_for_selector("#builder", timeout=5000)
    page.wait_for_function("document.querySelectorAll('#b-plats .chip').length > 0 || document.querySelector('#b-count').textContent", timeout=30000)


def build_smoke_set(page: Page) -> int:
    goto_view(page, "sets")
    page.click("#sets-new")
    page.wait_for_function("location.hash === '#sets/edit' && document.querySelector('#set-title').textContent === 'New playlist'", timeout=5000)
    page.wait_for_selector("#set-editor", state="visible", timeout=5000)
    check(page.is_visible("#set-name") and page.locator("#set-empty:not([hidden])").count() == 1,
          "playlists: + New playlist opens an empty editor with a Name field and a hint")
    open_builder(page)
    # The base-playlist select fills when the root listing answers, which a
    # busy device can do late; "Add 0 GIFs" before that is not a count.
    page.wait_for_function("document.querySelectorAll('#b-base option').length > 0", timeout=30000)
    for p in ("MD", "SNES"):
        page.click(f"#b-plats [data-platform={p}]")
    page.wait_for_function("/^Add [1-9][\\d,]* GIFs?$/.test(document.querySelector('#b-append').textContent)"
                           " && document.querySelector('#b-use').textContent.startsWith('Replace with ')", timeout=30000)
    page.click("#b-use")
    page.wait_for_selector("#builder", state="detached", timeout=5000)
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


def get_text(url: str) -> str:
    with urllib.request.urlopen(url, timeout=30) as r:
        return r.read().decode()


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


def settle(base: str, n: int) -> None:
    """Wait for the playlist in play to be reindexed with `n` overrides."""
    deadline = time.time() + 10
    while time.time() < deadline and ((st := get_json(base + "/api/status"))["overrides"] != n or st.get("index_state") == "building"):
        time.sleep(0.1)
    time.sleep(0.5)


def now_fav_checks(page: Page, base: str) -> None:
    """Now's ★ (the mock paused): the GIF on screen a favourite in the
    playlist playing, as the Library's Favourite in… does, and off again.
    Automatic preparing is off meanwhile: a long pause would let the mock's
    builder run and change what the cache checks later expect."""
    put_json(base + "/api/config", {"auto_prepare": "off"})
    page.wait_for_selector("#now-fav:not([hidden])", timeout=10000)
    path = page.text_content("#now-path")
    tip = page.get_attribute("#now-fav", "title")
    check(page.inner_text("#now-fav") == "☆" and tip == "Favourite in “All files”" and page.get_attribute("#now-fav", "aria-pressed") == "false",
          f"now fav: ☆ for {path.rsplit('/', 1)[-1]} ({tip!r})")
    page.click("#now-fav")
    # ★ and aria-pressed in one look, as the click left them: the save
    # reindexes the playlist, and the panel moving on to the next GIF then
    # (right) puts ☆ back before the overrides are read below.
    page.wait_for_function("(b => b.textContent === '★' && b.getAttribute('aria-pressed') === 'true')(document.querySelector('#now-fav'))")
    r = wait_overrides(base, "", 1)
    check(r["overrides"] == [{"path": path, "w": 3}], f"now fav: ★ (pressed) -> GET /api/overrides?set= {r['overrides']}")
    # The save reindexes the playlist, which can move the mock on: back to the GIF.
    settle(base, 1)
    if get_json(base + "/api/status")["now_playing"]["path"] != path:
        post_json(base + "/api/play", {"path": path})
    page.wait_for_function(f"document.querySelector('#now-path').textContent === {json.dumps(path)} && document.querySelector('#now-fav').textContent === '★'", timeout=10000)
    page.click("#now-fav")
    page.wait_for_function("document.querySelector('#now-fav').textContent === '☆'")
    deadline = time.time() + 10
    while time.time() < deadline and get_json(base + "/api/overrides?set=")["overrides"]:
        time.sleep(0.1)
    check(not get_json(base + "/api/overrides?set=")["overrides"], "now fav: ☆ again -> no overrides left")
    # Clicked while the list is still being read (slowed here): the others survive.
    # (The last two of the playlist: the reindex the seeding causes may move what is on screen.)
    others = card_gifs()[-2:]
    seeded = [{"path": others[0], "never": True}, {"path": others[1], "loops": 3}]
    put_text(base + "/api/files/filelist.meta", "".join(render_meta_line(o["path"], o) + "\n" for o in seeded))
    settle(base, 2)
    path = get_json(base + "/api/status")["now_playing"]["path"]
    goto_view(page, "library")
    page.evaluate("import('./js/state.js').then(({ app }) => { const f = app.src.overrides.bind(app.src); app.src._ov = f; app.src.overrides = (s) => new Promise((r) => setTimeout(r, 2500)).then(() => f(s)); })")
    page.evaluate("location.hash = '#now'")
    page.wait_for_function(f"document.querySelector('#now-path').textContent === {json.dumps(path)}", timeout=2000)
    page.click("#now-fav")
    page.wait_for_function("document.querySelector('#now-fav').textContent === '★'")
    r = wait_overrides(base, "", 3)
    got = sorted(r["overrides"], key=lambda o: o["path"])
    check(got == sorted(seeded + [{"path": path, "w": 3}], key=lambda o: o["path"]), f"now fav: a click during the read keeps the others ({got})")
    page.evaluate("import('./js/state.js').then(({ app }) => { app.src.overrides = app.src._ov; })")
    settle(base, 3)
    if get_json(base + "/api/status")["now_playing"]["path"] != path:
        post_json(base + "/api/play", {"path": path})
    page.wait_for_function(f"document.querySelector('#now-path').textContent === {json.dumps(path)} && document.querySelector('#now-fav').textContent === '★'", timeout=10000)
    page.click("#now-fav")
    page.wait_for_function("document.querySelector('#now-fav').textContent === '☆'")
    deadline = time.time() + 10  # its save landed (two left), then the file goes
    while time.time() < deadline and len(get_json(base + "/api/overrides?set=")["overrides"]) != 2:
        time.sleep(0.1)
    page.wait_for_timeout(500)
    req = urllib.request.Request(base + "/api/files/filelist.meta", method="DELETE")
    urllib.request.urlopen(req, timeout=30).close()
    deadline = time.time() + 10
    while time.time() < deadline and get_json(base + "/api/overrides?set=")["overrides"]:
        time.sleep(0.1)
    put_json(base + "/api/config", {"auto_prepare": "idle"})


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
    const m = (await import('./js/web.js')).default;  // the app's instance (js/wasm.js)
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
    """Settings -> Playback: choose a look and glow (saved as chosen); status
    and config must report them."""
    goto_view(page, "settings/look")
    page.wait_for_selector("#st-look", timeout=20000)
    page.select_option("#st-look", look)
    page.select_option("#st-glow", glow)
    cfg = wait_config(base, lambda c: c.get("look") == look and c.get("glow") == glow)
    deadline = time.time() + 10
    st = get_json(base + "/api/status")
    while (st.get("look"), st.get("glow")) != (look, glow) and time.time() < deadline:
        time.sleep(0.2)
        st = get_json(base + "/api/status")
    check((cfg.get("look"), cfg.get("glow")) == (look, glow) and (st.get("look"), st.get("glow")) == (look, glow),
          f"settings: chosen (saved as changed) -> look {look}, glow {glow} in /api/config and /api/status "
          f"({st.get('look')}, {st.get('glow')}, glow_held {st.get('glow_held')})")


def look_checks(page: Page, base: str, mock: bool) -> None:
    """The Playback card's look and glow selects, their API validation, and
    the Now preview wearing the look. Leaves the original values saved."""
    goto_view(page, "settings/look")
    page.wait_for_selector("#st-look", timeout=20000)
    opts = page.eval_on_selector_all("#st-look option", "els => els.map(e => [e.value, e.textContent])")
    check(opts == [["colour", "Full colour"], ["amber", "Amber"], ["red", "Red"], ["green", "Green"], ["blue", "Blue"], ["white", "White"]],
          f"settings: #st-look options {opts}")
    check(page.eval_on_selector_all("#st-glow option", "els => els.map(e => e.value)") == ["off", "short", "long"],
          "settings: #st-glow options off/short/long")
    cfg0 = get_json(base + "/api/config")
    st0 = get_json(base + "/api/status")
    # (No look_formula_name, look_blend or look_amount: the Formula's own
    # `name` and `look` lines say them, formula-apps F2.)
    check(list(cfg0)[-15:] == ["attract", "look", "glow", "look_formula", "boot_show",
                              "panel_sync", "panel_split", "formula_jit", "formula_arena", "formula_keep", "api_key_set", "panel", "panel_now", "panel_next", "panel_trial_s"]
          and not {"look_formula_name", "look_blend", "look_amount"} & set(cfg0),
          f"config: attract/look/glow/look_formula/boot_show/panel_sync/panel_split/"
          f"formula_jit/formula_arena/formula_keep/api_key_set, then the panel keys, end the object ({list(cfg0)[-15:]})")
    # The mock's config is the firmware's (http.rs settings_json's keys; the
    # mock's panel keys are the panel-timing card's).
    fw = re.findall(r'"(\w+)":', "".join(re.findall(r'r#"(.*?)"#', (REPO / "firmware/src/http.rs").read_text().split("fn settings_json", 1)[1].split("\n}\n", 1)[0])))
    mock_keys = set(cfg0) - {"panel", "panel_now", "panel_next", "panel_trial_s"}
    check(len(fw) > 20 and set(fw) == mock_keys and "look_formula" in fw,
          f"config: the mock's keys are the firmware's settings_json ({len(fw)}; only in the firmware {sorted(set(fw) - mock_keys)}, only in the mock {sorted(mock_keys - set(fw))})")
    check(list(st0)[-9:] == ["look", "glow", "glow_held", "formula_look", "attract_live", "max_brightness",
                             "brightness_safe_max", "safe_start_until", "scan"],
          f"status: look/glow/glow_held/formula_look/attract_live/max_brightness/brightness_safe_max/safe_start_until/scan"
          f" end the object ({list(st0)[-9:]})")
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
    goto_view(page, "settings/look")
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
        deadline = time.time() + 10
        while get_json(base + "/api/config")["boot_show"] != other and time.time() < deadline:
            time.sleep(0.2)
        check(get_json(base + "/api/config")["boot_show"] == other, f"settings: start-up {other} saved as chosen")
        page.select_option("#st-boot", orig)
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
    wait_config(base, lambda c: c["boot_show"] == orig)
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

    def at(sel):
        # Brightness and its maximum are on Playback: open the tab first.
        t = "play"
        if not page.locator(f"#st-tab-{t}").count():
            page.click(f"#st-tabs [data-tab={t}]")
            page.wait_for_selector(f"#st-tab-{t}", timeout=10000)
        return sel

    def slider_max():
        return int(page.eval_on_selector(at("#st-brightness"), "e => e.dataset.ceiling"))

    def reload_settings():
        # The page reads the config once; a PUT made behind its back needs a reload.
        goto_view(page, "settings/play")
        page.reload()
        page.wait_for_selector(at("#st-maxbright"), state="attached", timeout=20000)

    orig = cfg()
    safe = get_json(base + "/api/status")["brightness_safe_max"]
    st = get_json(base + "/api/status")
    check(st["max_brightness"] == orig["max_brightness"] and isinstance(safe, int),
          f"power: status carries max_brightness {st['max_brightness']} and brightness_safe_max {safe}")
    try:
        put_json(base + "/api/config", {"max_brightness": safe, "brightness": 100})
        reload_settings()
        check(slider_max() == safe, f"power: the brightness slider's ceiling is {safe} (follows max_brightness)")
        page.eval_on_selector(at("#st-brightness"), "e => { e.value = 255; e.dispatchEvent(new Event('input')); }")
        check(int(page.input_value(at("#st-brightness"))) == safe, "power: the slider stops at the ceiling")
        page.eval_on_selector(at("#st-brightness"), "e => { e.value = 100; e.dispatchEvent(new Event('input')); }")
        wait_config(base, lambda c: c["brightness"] == 100)
        check(page.input_value(at("#st-maxbright")) == str(safe), "power: the control shows max_brightness")
        check(not page.is_visible(at("#st-maxbright-warn")), "power: no warning at the tested limit")
        text = page.get_attribute(at("#st-maxbright-note .q"), "title") or ""
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
        check(page.is_visible(at("#st-maxbright")) and page.eval_on_selector("#st-tab-play", "t => !!t.querySelector('#st-brightness') && t.querySelector('#st-brightness').compareDocumentPosition(t.querySelector('#st-maxbright')) & 4"),
              "power: Maximum brightness is on Playback, under the Brightness slider")
        check(page.is_visible(at("#st-maxbright-warn")), "power: warning shown above the tested limit")
        w = page.inner_text(at("#st-maxbright-warn"))
        check(w == f"Above the tested limit ({safe}). The display may reset. Only raise this with a stronger power supply",
              f"power: warning text ({w})")
        at("#st-power-card") and page.evaluate("document.querySelector('#st-power-card').scrollIntoView({block: 'center'})")
        shot(page, "settings-power", "laptop")
        # The control: lower the max below the brightness; both saved.
        page.fill(at("#st-maxbright"), str(safe - 60))
        check(not page.is_visible(at("#st-maxbright-warn")), "power: warning hides when the control drops to the safe value")
        page.dispatch_event(at("#st-maxbright"), "change")
        deadline = time.time() + 10
        while cfg()["max_brightness"] != safe - 60 and time.time() < deadline:
            time.sleep(0.2)
        c = cfg()
        check(c["max_brightness"] == safe - 60 and c["brightness"] == safe - 60,
              f"power: lowering max lowers brightness in the same save ({c['brightness']}/{c['max_brightness']})")
        at("#st-brightness")
        page.wait_for_function("+document.getElementById('st-brightness').dataset.ceiling == %d" % (safe - 60), timeout=5000)
        check(int(page.input_value(at("#st-brightness"))) == safe - 60, "power: the slider follows the lowered brightness")
        # Typing above the safe value shows the warning before saving.
        page.fill(at("#st-maxbright"), str(safe + 1))
        check(page.is_visible(at("#st-maxbright-warn")), "power: warning shows as soon as the control is above the safe value")
        page.fill(at("#st-maxbright"), str(safe))
        check(not page.is_visible(at("#st-maxbright-warn")), "power: warning hidden at the safe value")
        page.fill(at("#st-maxbright"), str(safe - 60))
        page.dispatch_event(at("#st-maxbright"), "change")
        # Round trip through the Playback slider too (saved when it settles).
        page.eval_on_selector(at("#st-brightness"), "e => { e.value = 40; e.dispatchEvent(new Event('input')); }")
        c = wait_config(base, lambda c: c["brightness"] == 40)
        check(c["brightness"] == 40 and c["max_brightness"] == safe - 60, "power: the brightness slider leaves the maximum alone")
    finally:
        put_json(base + "/api/config", {"max_brightness": orig["max_brightness"]})
        put_json(base + "/api/config", {"brightness": orig["brightness"]})
    c = cfg()
    check((c["brightness"], c["max_brightness"]) == (orig["brightness"], orig["max_brightness"]),
          "power: brightness and maximum left as found")
    no_leaks(page, "settings (power)")
    goto_view(page, "now")


PANEL_TRIAL_S = 6  # the mock's --panel-trial in the mock pass (the firmware's is 60)
PANEL_DEFAULTS = {"clk_phase": True, "latch_blanking": 1, "driver": "shiftreg", "i2s_speed": 8, "min_refresh": 60}


def panel_checks(page: Page, base: str) -> None:
    """Settings -> Device -> Panel timing (js/panel.js, loaded with
    the Device tab): the section, the API's 400s/409, a live trial kept, undone and
    left to revert, a driver change through a restart and undone, and Reset to
    defaults. Mock only (the mock's trial is PANEL_TRIAL_S)."""
    def cfg():
        return get_json(base + "/api/config")

    def panel_of(c):
        return {k: c[k] for k in ("panel", "panel_now", "panel_next", "panel_trial_s")}

    c = cfg()
    check(c["panel"] == PANEL_DEFAULTS and c["panel_now"] == PANEL_DEFAULTS and c["panel_next"] is None
          and c["panel_trial_s"] is None, f"panel: /api/config carries the defaults ({panel_of(c)})")
    for bad, code in (({"panel": {"latch_blanking": 0}}, 400), ({"panel": {"latch_blanking": 5}}, 400),
                      ({"panel": {"i2s_speed": 10}}, 400), ({"panel": {"min_refresh": 241}}, 400),
                      ({"panel": {"driver": "ws2812"}}, 400), ({"panel": {"clkphase": False}}, 400),
                      ({"panel": {"clk_phase": "no"}}, 400), ({"panel": {"clk_phase": False}, "panel_confirm": False}, 400),
                      ({"panel_confirm": True}, 409)):
        check(put_status(base, bad) == code, f"panel: {json.dumps(bad)} refused ({code})")
    check(panel_of(cfg()) == panel_of(c), "panel: nothing changed by the refused requests")

    goto_view(page, "settings/device")
    page.wait_for_selector("#st-panel:not([hidden]) #pt-driver", timeout=20000)
    # Moving pictures: both on by default; the second greys out with the
    # first off, and each saves at once.
    page.wait_for_selector("#st-sync:not([hidden]) #ps-sync", timeout=5000)
    check(page.is_checked("#ps-sync") and page.is_checked("#ps-split") and page.is_enabled("#ps-split"),
          "sync: both on by default, the second enabled")
    check("Sync to the panel’s refresh (stops tearing)" in page.inner_text("#st-sync"), "sync: the plain label")
    page.uncheck("#ps-sync")
    page.wait_for_function("() => document.querySelector('#ps-split').disabled", timeout=5000)
    check(cfg()["panel_sync"] is False and cfg()["panel_split"] is True, "sync: off saved, split kept")
    page.check("#ps-sync")
    page.wait_for_function("() => !document.querySelector('#ps-split').disabled", timeout=5000)
    check(cfg()["panel_sync"] is True, "sync: on again")
    text = page.inner_text("#st-panel")
    check("Panel timing" in text and "If some LEDs show wrong colours on certain pictures, press Test panel timing, or try these one at a time" in text,
          "panel: the section and its explanation are under Device")
    vals = {k: page.input_value(f"#pt-{k}") for k in PANEL_DEFAULTS}
    check(vals == {k: json.dumps(v) for k, v in PANEL_DEFAULTS.items()}, f"panel: the selects show the defaults ({vals})")
    check(page.locator("#pt-driver option:checked").inner_text() == "Standard (default)", "panel: the default is marked")
    check(not page.is_visible("#pt-bar"), "panel: no banner with nothing on trial")

    # A live trial, kept.
    page.select_option("#pt-clk_phase", "false")
    c = wait_config(base, lambda c: c["panel_trial_s"] is not None)
    check(c["panel_now"]["clk_phase"] is False and c["panel"]["clk_phase"] is True,
          f"panel: choosing Falling tries it, unsaved ({panel_of(c)})")
    page.wait_for_selector("#pt-keep", state="visible", timeout=5000)
    check(re.search(r"go back in \d+ s unless you keep them", page.inner_text("#pt-bar")) is not None,
          f"panel: the trial banner counts down ({page.inner_text('#pt-bar')!r})")
    page.click("#pt-keep")
    c = wait_config(base, lambda c: c["panel_trial_s"] is None)
    check(c["panel"]["clk_phase"] is False and c["panel_now"]["clk_phase"] is False, "panel: Keep saves the trial")
    page.wait_for_selector("#pt-bar", state="hidden", timeout=5000)

    # A live trial, undone.
    page.select_option("#pt-latch_blanking", "3")
    wait_config(base, lambda c: c["panel_trial_s"] is not None)
    page.wait_for_selector("#pt-undo", state="visible", timeout=5000)
    page.click("#pt-undo")
    c = wait_config(base, lambda c: c["panel_trial_s"] is None)
    check(c["panel_now"]["latch_blanking"] == 1 and c["panel"]["latch_blanking"] == 1 and c["panel_now"]["clk_phase"] is False,
          f"panel: Undo goes back to the saved settings ({panel_of(c)})")
    page.wait_for_function("document.querySelector('#pt-latch_blanking').value === '1'", timeout=5000)

    # A live trial left alone: it reverts by itself.
    page.select_option("#pt-latch_blanking", "2")
    wait_config(base, lambda c: c["panel_trial_s"] is not None)
    c = wait_config(base, lambda c: c["panel_trial_s"] is None, timeout_s=PANEL_TRIAL_S + 5)
    check(c["panel_now"]["latch_blanking"] == 1, f"panel: an unconfirmed trial reverts after {PANEL_TRIAL_S} s")
    page.wait_for_function("document.querySelector('#pt-latch_blanking').value === '1'", timeout=8000)
    check(not page.is_visible("#pt-bar"), "panel: the page follows the revert")

    # A driver change: staged for a restart, tried after it, undone by another.
    page.select_option("#pt-driver", '"fm6126a"')  # (option values are JSON)
    c = wait_config(base, lambda c: c["panel_next"] is not None)
    check(c["panel_next"]["driver"] == "fm6126a" and c["panel_now"]["driver"] == "shiftreg" and c["panel_trial_s"] is None,
          f"panel: a driver change waits for a restart ({panel_of(c)})")
    page.wait_for_selector("#pt-restart", state="visible", timeout=5000)
    check("after a restart" in page.inner_text("#pt-bar"), "panel: the banner says it takes a restart")
    check(put_json(base + "/api/config", {"panel": {"min_refresh": 90}}).get("reboot_required") is True,
          "panel: PUT reply says reboot_required for a boot value")
    page.click("#pt-restart")
    c = wait_config(base, lambda c: c["panel_trial_s"] is not None)
    check(c["panel_now"]["driver"] == "fm6126a" and c["panel_now"]["min_refresh"] == 90 and c["panel"]["driver"] == "shiftreg",
          f"panel: after the restart the staged settings run on trial ({panel_of(c)})")
    page.wait_for_selector("#pt-undo", state="visible", timeout=10000)
    check(page.input_value("#pt-driver") == '"fm6126a"', "panel: the page shows the trial's driver")
    expected.append("/api/")  # the API is down through the simulated restart
    page.click("#pt-undo")
    deadline = time.time() + 15
    while True:  # (503s while it "restarts")
        try:
            c = cfg()
            if (c["panel_trial_s"] is None and c["panel_next"] is None) or time.time() > deadline:
                break
        except urllib.error.HTTPError:
            pass
        time.sleep(0.3)
    check(c["panel_now"]["driver"] == "shiftreg" and c["panel_now"]["min_refresh"] == 60,
          f"panel: Undo of a restart trial restarts on the saved settings ({panel_of(c)})")
    page.wait_for_function("document.querySelector('#pt-driver').value === '\"shiftreg\"'", timeout=15000)
    page.wait_for_selector("#pt-bar", state="hidden", timeout=15000)
    expected.remove("/api/")

    # Reset to defaults (the saved Falling edge goes too).
    page.click("#pt-reset")
    c = wait_config(base, lambda c: c["panel"] == PANEL_DEFAULTS)
    check(c["panel"] == PANEL_DEFAULTS and c["panel_now"] == PANEL_DEFAULTS and c["panel_next"] is None,
          f"panel: Reset to defaults saves and applies the defaults ({panel_of(c)})")
    page.wait_for_function("document.querySelector('#pt-clk_phase').value === 'true'", timeout=5000)
    page.locator("#st-panel").screenshot(path=str(OUT / "settings-panel-laptop.png"))
    panel_test_checks(page, base)


def panel_test_checks(page: Page, base: str) -> None:
    """Test panel timing (js/paneltest.js, POST /api/panel/test): the API's
    400s and the 409 during a trial, then the guided flow — B picked and
    saved, both stages with dots, "the same", Stop. Mock only; starts and
    ends on the defaults."""
    def post_status(obj) -> int:
        try:
            post_json(base + "/api/panel/test", obj)
            return 200
        except urllib.error.HTTPError as e:
            return e.code

    for bad in ({}, {"setting": "driver"}, {"setting": "clk_phase", "stop": True}, {"stop": False}, {"clkphase": 1}):
        check(post_status(bad) == 400, f"panel test: {json.dumps(bad)} refused (400)")
    r = post_json(base + "/api/panel/test", {"setting": "clk_phase"})
    check(r.get("a", {}).get("clk_phase") is True and r.get("b", {}).get("clk_phase") is False and r.get("left_s") == 120,
          f"panel test: clk_phase is A rising, B falling, 120 s ({r})")
    r = post_json(base + "/api/panel/test", {"setting": "latch_blanking"})
    check((r["a"]["latch_blanking"], r["b"]["latch_blanking"]) == (1, 3), f"panel test: latch blanking 1 against 3 ({r})")
    check(post_json(base + "/api/panel/test", {"stop": True}) == {"ok": True}, "panel test: stop")
    put_json(base + "/api/config", {"panel": {"latch_blanking": 2}})
    check(post_status({"setting": "clk_phase"}) == 409, "panel test: refused during a trial (409)")
    put_json(base + "/api/config", {"panel_confirm": False})
    put_json(base + "/api/config", {"panel": {"driver": "fm6126a"}})
    check(post_status({"setting": "latch_blanking"}) == 409, "panel test: refused with a change waiting for a restart (409)")
    put_json(base + "/api/config", {"panel_confirm": False})
    check(get_json(base + "/api/config")["panel"] == PANEL_DEFAULTS, "panel test: the API calls saved nothing")

    def start(question: str) -> None:
        page.click("#pt-test")
        page.wait_for_function("(q) => document.querySelector('#pt-box:not([hidden])')?.innerText.includes(q)",
                               arg=question, timeout=5000)

    def answer(button: str, said: str) -> None:
        page.locator("#pt-box button", has_text=button).click()
        page.wait_for_function("(q) => document.querySelector('#pt-box').innerText.includes(q)", arg=said, timeout=5000)

    check(page.is_visible("#pt-test") and not page.is_visible("#pt-box"), "panel test: the button, no box yet")
    start("Watch the panel. Does A or B look cleaner?")
    page.locator("#st-panel").screenshot(path=str(OUT / "settings-panel-test-laptop.png"))
    answer("B looks clean", "Saved: Clock edge Falling.")
    c = wait_config(base, lambda c: c["panel"]["clk_phase"] is False)
    check(c["panel"]["clk_phase"] is False and c["panel_trial_s"] is None and c["panel_now"]["clk_phase"] is False,
          f"panel test: B is saved at once, no trial ({c['panel']})")
    page.wait_for_function("document.querySelector('#pt-clk_phase').value === 'false'", timeout=5000)
    start("Does A or B look cleaner?")
    answer("Both have dots", "One more setting.")
    answer("Both have dots", "Try Driver chip below")
    start("Does A or B look cleaner?")
    answer("Both look the same / fine", "Nothing changed.")
    start("Does A or B look cleaner?")
    page.locator("#pt-box button", has_text="Stop").click()
    page.wait_for_selector("#pt-box", state="hidden", timeout=5000)
    c = get_json(base + "/api/config")
    check(c["panel"] == dict(PANEL_DEFAULTS, clk_phase=False) and c["panel_trial_s"] is None,
          f"panel test: only the pick was saved ({c['panel']})")
    # A trial started elsewhere closes the box.
    start("Does A or B look cleaner?")
    page.select_option("#pt-latch_blanking", "2")
    page.wait_for_selector("#pt-box", state="hidden", timeout=8000)
    check(not page.is_visible("#pt-box"), "panel test: a trial closes the box")
    put_json(base + "/api/config", {"panel_confirm": False})
    # Leaving the Device tab, or Settings, ends a test in progress (its 100 s
    # re-POST would otherwise keep the test pictures up).
    page.evaluate("async () => { const { app } = await import('/js/state.js'); await app.refreshConfig(); }")
    stops: list[str] = []
    on_req = lambda r: r.method == "POST" and "/api/panel/test" in r.url and "stop" in (r.post_data or "") and stops.append(r.url)
    page.on("request", on_req)
    for away in ("settings/look", "now"):
        start("Does A or B look cleaner?")
        n = len(stops)
        goto_view(page, away)
        page.wait_for_timeout(500)
        goto_view(page, "settings/device")
        page.wait_for_selector("#pt-test", timeout=10000)
        check(len(stops) == n + 1 and not page.is_visible("#pt-box"),
              f"panel test: leaving for #{away} stops the test ({len(stops) - n} stop) and closes the box")
    page.remove_listener("request", on_req)
    # A chip that forces the rising edge: no clock-edge test; the page
    # starts at latch blanking.
    put_json(base + "/api/config", {"panel": {"driver": "mbi5124"}, "panel_confirm": True})
    expected.append("/api/")  # down through the restart that applies it
    post_json(base + "/api/reboot", {})
    deadline = time.time() + 20
    while time.time() < deadline:
        try:
            if get_json(base + "/api/config")["panel_now"]["driver"] == "mbi5124":
                break
        except urllib.error.HTTPError:
            pass
        time.sleep(0.3)
    expected.remove("/api/")
    check(post_status({"setting": "clk_phase"}) == 409, "panel test: clk_phase refused on MBI5124 (409)")
    page.evaluate("async () => { const { app } = await import('/js/state.js'); await app.refreshConfig(); }")
    start("Watch the panel. Does A or B look cleaner?")
    check("One more setting" not in page.inner_text("#pt-box"), "panel test: MBI5124 starts at latch blanking, as the first question")
    answer("B looks clean", "Saved: Latch blanking 3.")
    put_json(base + "/api/config", {"panel": None})


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
    page.wait_for_function(f"{TREE}?.folders.some(f => f.cat === '@')", timeout=30000)
    opts = page.evaluate(f"{TREE}.folders.map(f => [f.cat, f.name])")
    # System lists (a leading "_", e.g. a card's _bench) sort after Live.
    real = [o for o in opts if not o[0].startswith("_")]
    check(real[-1:] == [["@", "Live"]] and len(real) > 2 and not any(o[0] == "@fx" or o[1] == "Effects" for o in opts),
          f"library: the folders end with Live, no Effects folder (system lists after) ({opts[-3:]})")
    open_live(page)
    rows = live_rows(page)
    want = ["@clock", "@clock?seconds=1", "@clock?date=1", "@clock?style=small&seconds=1&date=1", "@text?msg=OpenDMD"]
    check(rows == want, f"library: Live lists five rows ({rows})")
    titles = page.eval_on_selector_all("#lib-list .vrow:not([hidden]) .t", "els => els.map(e => e.textContent)")
    check(len(set(titles)) == 5 and titles[1] == "Clock with seconds", f"library: the Live rows have names of their own ({titles})")
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
        page.wait_for_function(f"document.querySelector('#lib-play').dataset.path === {json.dumps(path)}", timeout=10000)
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


def no_effects_checks(page: Page, base: str, mock: bool) -> None:
    """The built-in effects are gone: the Library has the Live folder and no
    Effects folder, tab or fx- form (nor has the playlist editor's picker);
    on the mock, POST /api/play of an @fx: path is a 415 naming Formulas,
    attract "fx:.." a 400, and a playlist with an old @fx: line still plays
    its other entries."""
    goto_view(page, "library")
    page.wait_for_function(f"{TREE}?.folders.length > 2", timeout=30000)
    pick_cat(page, "@")
    folders = page.evaluate(f"{TREE}.folders.map(f => [f.cat, f.name])")
    labels = page.eval_on_selector_all("#lib-list .vrow.dir, #lib-list [role=treeitem], #lib-list .folder", "els => els.map(e => e.textContent.trim())")
    fx_ids = page.eval_on_selector_all("[id^='fx-']", "els => els.map(e => e.id)")
    check(any(f[0] == "@" and f[1] == "Live" for f in folders) and not any(f[0] == "@fx" or f[1] == "Effects" for f in folders),
          f"library: the Live folder and no Effects folder ({folders[-3:]})")
    check(not any("Effects" in t for t in labels) and fx_ids == [] and "Effects" not in page.inner_text("#view-library"),
          f"library: no tree folder labelled Effects, no fx- element, no Effects tab ({fx_ids})")
    # The playlist editor's picker has only Live (no Effects).
    goto_view(page, "sets")
    picker = page.eval_on_selector_all("#set-cat option, #set-cats button, #set-cats a", "els => els.map(e => e.textContent.trim())")
    check("Effects" not in picker and "Effects" not in page.inner_text("#view-sets"), f"playlists: no Effects in the editor's picker ({picker})")
    pl = page.evaluate("import('./js/state.js').then(m => ['/sets/Sprites_and_Clock.txt', '/sets/_x.txt', '/filelist.hall'].map(m.playlistLabel))")
    check(pl == ["Sprites and Clock", "_x", "Halloween"], f"playlists: names on the card read as words ({pl})")
    no_leaks(page, "library (no effects)")
    if not mock:
        return
    expected.append("/api/play")
    for path in ("@fx:tunnel", "@fx:starfield?warp=0&fps=25", "@fx:psi?src=gray(x)"):
        try:
            post_json(base + "/api/play", {"path": path})
            check(False, f"api: POST /api/play {path} refused")
        except urllib.error.HTTPError as e:
            msg = json.loads(e.read().decode()).get("error", "")
            check(e.code == 415 and "Formulas replace them" in msg, f"api: POST /api/play {path} -> {e.code} '{msg}' (415, Formulas replace them)")
    expected.remove("/api/play")
    for bad in ({"attract": "fx:nonsense"}, {"attract": "fx:tunnel?palette=fire"}):
        check(put_status(base, bad) == 400, f"settings: {bad} -> 400")
    # A playlist with an old @fx: line between a GIF and a clock: the line is
    # skipped, the others stay (2 entries) and play.
    gif = next(f"/gifs/Consoles/{f['name']}" for f in get_json(base + "/api/files?dir=/gifs/Consoles")["entries"]
               if f["name"].lower().endswith(".gif"))
    before = get_json(base + "/api/config").get("set", "")
    put_text(base + "/api/files/sets/fxold.txt", f"{gif}\n@fx:tunnel\n@clock\n")
    try:
        put_json(base + "/api/config", {"set": "fxold"})
        st = get_json(base + "/api/status")
        check(st.get("entries") == 2, f"playlists: an @fx: line is skipped, the other two entries stay ({st.get('entries')})")
        expected.append("/api/play")
        post_json(base + "/api/play", {"path": "@clock"})
        cur = wait_playing(base, "@clock")
        expected.remove("/api/play")
        check(cur == "@clock", f"playlists: the playlist with the retired line still plays its clock ({cur})")
    finally:
        put_json(base + "/api/config", {"set": before})
        urllib.request.urlopen(urllib.request.Request(base + "/api/files/sets/fxold.txt", method="DELETE"), timeout=30).read()


def sim_png(root: Path, look: str, out: Path, path: str = "@text?msg=OpenDMD&scroll=0", at_ms: int | None = None) -> bytes:
    """The desktop simulator's canvas for `@text?msg=OpenDMD&scroll=0` after
    `look` (or `at_ms` into it), as 128x32 RGBA bytes."""
    from PIL import Image
    (root / "gifs").mkdir(parents=True, exist_ok=True)
    cmd = ["cargo", "run", "--release", "-q", "--", "--root", str(root), "--play", path,
           "--screenshot-canvas", str(out)]
    if look != "colour":
        cmd += ["--look", look]
    if at_ms is not None:
        cmd += ["--screenshot-at", str(at_ms)]
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


def www_pack_paths(pack: bytes) -> list[str]:
    """The paths in a www pack's entry table (docs/www-pack.md)."""
    assert pack[:4] == b"ODWP", "not a www pack"
    n = int.from_bytes(pack[16:18], "little")
    out = []
    for i in range(n):
        e = pack[64 + 16 * i:80 + 16 * i]
        off, ln = int.from_bytes(e[0:4], "little"), int.from_bytes(e[12:14], "little")
        out.append(pack[off:off + ln].decode())
    return out


def api_page_checks(page: Page, base: str) -> None:
    """The API reference lives on the project site, not in the pack:
    Settings -> About links this firmware's own page there
    (https://opendmd.dev/dev/api/v<version>.html, frozen when that
    release was published) when the player runs a release (its firmware a
    release number and its web app the same release, `v<version>`), else
    the latest (dev/api.html): this build's web app is a `git describe`
    and the mock's firmware 0.1.0-mock; then /api/status rewritten to a
    release, and to a firmware version that is no release number. A new
    tab, never the app's own. The www pack has no api.html, and the mock
    serves none. (tools/api-doc-check.py, run by `make web`, holds
    docs/api.md's routes to http.rs.)
    And beside it, the third-party licences: licenses.txt, served as text.
    """
    site = "https://opendmd.dev/"
    st = get_json(base + "/api/status")
    link = page.eval_on_selector("#st-api", "a => [a.getAttribute('href'), a.target, a.rel, a.textContent]")
    check(link == [site + "dev/api.html", "_blank", "noopener", "API reference"],
          f"api link: a development build (firmware {st['version']}, web app {st['www_version']}) links the latest docs in a new tab ({link})")

    def as_status(**kw):
        def fn(route):
            r = route.fetch()
            j = r.json()
            route.fulfill(response=r, json={**j, **{k: v(j) for k, v in kw.items()}})
        return fn

    def href_becomes(want: str) -> str:
        try:
            page.wait_for_function("w => document.querySelector('#st-api')?.getAttribute('href') === w", arg=want, timeout=20000)
        except Exception:  # noqa: BLE001 (reported below)
            pass
        return page.eval_on_selector("#st-api", "a => a.getAttribute('href')")

    page.route("**/api/status", as_status(www_version=lambda j: "v" + j["version"]))
    try:
        want = f"{site}dev/api/v{st['version']}.html"
        got = href_becomes(want)
        check(got == want, f"api link: a release (firmware {st['version']}, web app v{st['version']}) links its own page ({got})")
    finally:
        page.unroute("**/api/status")
    page.route("**/api/status", as_status(version=lambda j: "1.0.0 local", www_version=lambda j: "v1.0.0 local"))
    try:
        got = href_becomes(site + "dev/api.html")
        check(got == site + "dev/api.html", f"api link: a firmware version that is no release number links the latest ({got})")
    finally:
        page.unroute("**/api/status")
    href_becomes(site + "dev/api.html")
    paths = www_pack_paths((REPO / "opendmd-web" / "www.pack").read_bytes())
    check("index.html" in paths and not any(p.startswith("api.html") for p in paths),
          f"api link: the www pack has no api.html ({len(paths)} entries)")
    code = status_of(urllib.request.Request(base + "/api.html"))[0]
    check(code == 404, f"api link: the mock serves no /api.html ({code})")
    # The third-party licences (licenses.txt: licenses/'s two notices, by
    # `make web`), linked beside it, served from the pack as text.
    link = page.eval_on_selector("#st-licences", "a => [a.getAttribute('href'), a.target, a.textContent]")
    check(link == ["./licenses.txt", "_blank", "Open-source licences"],
          f"licences: About links ./licenses.txt in a new tab ({link})")
    with urllib.request.urlopen(base + "/licenses.txt") as r:
        body = r.read()
        if r.headers.get("Content-Encoding") == "gzip":
            body = gzip.decompress(body)
        text, ctype = body.decode(), r.headers.get("Content-Type", "")
    check(ctype.startswith("text/plain") and "the OpenDMD firmware" in text and "the OpenDMD web app" in text
          and "ESP-IDF" in text and "wasm-bindgen" in text,
          f"licences: served as {ctype}, the firmware's and the web app's notices ({len(text):,} chars)")


OTA_VERSION = "9.9.9-smoke"
OTA_OLD_VERSION = "0.0.1"
UPD_VERSION = "10.0.0"   # "Check for updates": the release the fake site publishes
# js/firmware.js `older(a, b)`: (a, b, a is older).
OLDER_CASES = [("0.3.0", "0.3.1", True), ("0.3.1", "0.3.0", False), ("0.3.0", "0.3.0", False),
               ("0.10.0", "0.9.9", False), ("0.9.9", "0.10.0", True), ("1.0", "1.0.0", False),
               ("1.0.0-rc.1", "1.0.0", True), ("1.0.0", "1.0.0-rc.1", False), ("1.0.0-rc.2", "1.0.0-rc.10", True),
               ("v0.4.0", "0.3.9", False), ("0.0.1", "9.9.9-smoke", True), ("9.9.9-smoke", "0.3.3", False),
               ("0.3.13", "1.0.0", True), ("1.0.0", "0.3.13", False)]


def fake_app(path: Path, variant: str) -> None:
    """300 KB that pass for an app image of `variant`: 0xE9 first, an app
    descriptor (its magic word at 0x20, "<version> <variant>" at 0x30)."""
    b = bytearray(b"\xE9" + os.urandom(300_000))
    b[0x20:0x24] = (0xABCD5432).to_bytes(4, "little")
    b[0x30:0x50] = f"9.9.9-smoke {variant}".encode().ljust(32, b"\0")
    path.write_bytes(bytes(b))


def ota_bundles(tmp: Path) -> dict[str, Path]:
    """A throwaway key pair, a fake app image signed as OTA_VERSION, a copy
    of the bundle with one image byte flipped (tools/fw-sign.py), a file
    that is not a bundle at all, and bundles for the other flash layout
    (8mb: the mock is a 4mb player)."""
    sign = ["uv", "run", "--quiet", str(REPO / "tools" / "fw-sign.py")]
    subprocess.run([*sign, "keygen", "--key", str(tmp / "k"), "--pub", str(tmp / "p")], check=True, stdout=subprocess.DEVNULL)
    fake_app(tmp / "app.bin", "4mb")
    fake_app(tmp / "app8.bin", "8mb")
    good = tmp / "opendmd-9.9.9-smoke.odfw"
    subprocess.run([*sign, "sign", str(tmp / "app.bin"), "--version", OTA_VERSION, "--out", str(good), "--key", str(tmp / "k")],
                   check=True, stdout=subprocess.DEVNULL)
    b = bytearray(good.read_bytes())
    b[140 + 1000] ^= 1
    bad = tmp / "tampered.odfw"
    bad.write_bytes(bytes(b))
    junk = tmp / "notes.odfw"
    junk.write_bytes(b"not firmware at all " * 20)
    old = tmp / "opendmd-0.0.1.odfw"
    subprocess.run([*sign, "sign", str(tmp / "app.bin"), "--version", OTA_OLD_VERSION, "--out", str(old), "--key", str(tmp / "k")],
                   check=True, stdout=subprocess.DEVNULL)
    newer = tmp / f"opendmd-{UPD_VERSION}.odfw"
    subprocess.run([*sign, "sign", str(tmp / "app.bin"), "--version", UPD_VERSION, "--out", str(newer), "--key", str(tmp / "k")],
                   check=True, stdout=subprocess.DEVNULL)
    b = bytearray(newer.read_bytes())
    b[140 + 1000] ^= 1
    newer_bad = tmp / "newer-tampered.odfw"
    newer_bad.write_bytes(bytes(b))
    board8 = tmp / "opendmd-9.9.9-smoke-8mb.odfw"
    subprocess.run([*sign, "sign", str(tmp / "app8.bin"), "--version", OTA_VERSION, "--variant", "8mb", "--out", str(board8),
                    "--key", str(tmp / "k")], check=True, stdout=subprocess.DEVNULL)
    newer8 = tmp / f"opendmd-{UPD_VERSION}-8mb.odfw"
    subprocess.run([*sign, "sign", str(tmp / "app8.bin"), "--version", UPD_VERSION, "--variant", "8mb", "--out", str(newer8),
                    "--key", str(tmp / "k")], check=True, stdout=subprocess.DEVNULL)
    return {"pub": tmp / "p", "good": good, "bad": bad, "junk": junk, "old": old, "newer": newer, "newer_bad": newer_bad,
            "board8": board8, "newer8": newer8}


def status_of(req: urllib.request.Request) -> tuple[int, dict]:
    """The HTTP status and headers of a request, error or not."""
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            r.read()
            return r.status, dict(r.headers)
    except urllib.error.HTTPError as e:
        e.read()
        return e.code, dict(e.headers)


def fw_message(page: Page, pattern: str, timeout_ms: int = 30000) -> str:
    page.wait_for_function("p => new RegExp(p).test(document.querySelector('#fw-msg').textContent)", arg=pattern,
                           timeout=timeout_ms)
    return page.inner_text("#fw-msg")


FW_WATCH = """() => { window.__fw = []; window.__fwBar = 0;
  new MutationObserver(() => {
    window.__fw.push(document.querySelector('#fw-msg').textContent);
    window.__fwBar = Math.max(window.__fwBar, parseFloat(document.querySelector('#st-fw .bar i').style.width) || 0);
  }).observe(document.querySelector('#st-fw'), { subtree: true, childList: true, characterData: true, attributes: true }); }"""


def firmware_checks(page: Page, base: str, ota: dict[str, Path]) -> None:
    """Settings -> Device -> Firmware (js/firmware.js) against the mock's
    PUT /api/firmware, verified with the throwaway key."""
    goto_view(page, "settings/device")
    page.wait_for_selector("#fw-facts dd", timeout=20000)
    fw = get_json(base + "/api/firmware")
    check(sorted(fw) == ["build", "flash_size", "last", "log", "pending", "variant", "version"] and fw["pending"] is None
          and fw["variant"] == "4mb" and fw["flash_size"] == 4 << 20, f"firmware: GET /api/firmware {fw}")
    got = page.evaluate("async (cases) => { const { older } = await import('/js/firmware.js');"
                        " return cases.map(([a, b]) => older(a, b)); }", [[a, b] for a, b, _ in OLDER_CASES])
    wrong = [(a, b, g) for (a, b, w), g in zip(OLDER_CASES, got) if g != w]
    check(not wrong, f"firmware: older() on {len(OLDER_CASES)} version pairs (wrong: {wrong})")
    check(page.is_hidden("#fw-log") == (fw["log"] is None), "firmware: 'Update log' only when there is one")
    v0 = fw["version"]
    facts = page.inner_text("#fw-facts")
    check(v0 in facts and fw["build"] in facts and "DevKit (4 MB)" in facts and "This board has 8 MB" not in facts,
          f"firmware: the card shows {v0} and the board ({facts!r})")
    check(page.is_visible("#fw-go") and page.get_attribute("#fw-file", "accept") == ".odfw",
          "firmware: Update from file… (.odfw)")
    asked: list[str] = []
    record = lambda d: asked.append(d.message)  # noqa: E731 - new_page's handler accepts
    page.on("dialog", record)
    page.set_input_files("#fw-file", str(ota["junk"]))
    t = fw_message(page, "not an OpenDMD firmware file")
    check(not asked, f"firmware: a file that is not a bundle is refused in the page ('{t}')")
    # A bundle for the other flash layout: refused in the page, in words,
    # nothing sent; and the mock (as the firmware) refuses it too.
    page.set_input_files("#fw-file", str(ota["board8"]))
    t = fw_message(page, "^This file is for an 8 MB board in development")
    check(not asked and t == "This file is for the 8 MB build, and this player runs the DevKit (4 MB) firmware."
          " Use the DevKit (4 MB) update file", f"firmware: an 8mb bundle is refused in the page ('{t}')")
    req = urllib.request.Request(base + "/api/firmware", data=ota["board8"].read_bytes(), method="PUT",
                                 headers={"Content-Type": "application/octet-stream"})
    try:
        urllib.request.urlopen(req, timeout=30).read()
        code, why = 200, ""
    except urllib.error.HTTPError as e:
        code, why = e.code, json.loads(e.read() or b"{}").get("error", "")
    check(code == 422 and why.startswith("this update is for the 8 MB build and this player runs the DevKit (4 MB)")
          and get_json(base + "/api/firmware")["version"] == v0, f"firmware: PUT of an 8mb bundle -> {code} '{why}'")
    expected.append("/api/firmware")
    page.set_input_files("#fw-file", str(ota["bad"]))
    t = fw_message(page, "^The player refused this file")
    check("SHA-256 mismatch" in t and asked == [f"Install firmware {OTA_VERSION}? The panel goes dark for about a minute."],
          f"firmware: the tampered bundle is refused ('{t}'; asked {asked})")
    # An older bundle: asked differently; declined, nothing is sent.
    page.evaluate("() => { window.__confirm = window.confirm; window.__asked = [];"
                  " window.confirm = (m) => { window.__asked.push(m); return false; }; }")
    try:
        page.set_input_files("#fw-file", str(ota["old"]))
        page.wait_for_function("() => window.__asked.length > 0", timeout=10000)
    finally:
        older_asked = page.evaluate("() => { window.confirm = window.__confirm; return window.__asked; }")
    want = (f"Firmware {OTA_OLD_VERSION}: this is older than the firmware on the panel ({v0}). Install it anyway?"
            " The panel goes dark for about a minute.")
    check(older_asked == [want] and get_json(base + "/api/firmware")["version"] == v0 and not page.is_disabled("#fw-go"),
          f"firmware: an older bundle asks first, declined sends nothing ({older_asked})")
    check(get_json(base + "/api/firmware")["version"] == v0 and get_json(base + "/api/status")["version"] == v0,
          "firmware: refused -> the version is unchanged")
    check(not page.is_disabled("#fw-go"), "firmware: the button is back")
    page.evaluate(FW_WATCH)
    # The restart: the mock's API is a 503 for a few seconds.
    expected.append("/api/")
    try:
        page.set_input_files("#fw-file", str(ota["good"]))
        fw_message(page, "^Restarting… the panel comes back in about 45 s")
        check(page.is_disabled("#fw-go"), "firmware: 'Restarting…', the button waits")
        t = fw_message(page, f"^Updated to {re.escape(OTA_VERSION)}$", 60000)
        check(True, f"firmware: '{t}' after the mock's restart")
    finally:
        expected.remove("/api/")
        page.remove_listener("dialog", record)
    sending = [m for m in page.evaluate("window.__fw") if m.startswith("Sending…")]
    bar = page.evaluate("window.__fwBar")
    check("Sending… 100%" in sending and bar == 100, f"firmware: upload progress shown ({sending[0] if sending else None} … "
          f"{sending[-1] if sending else None}, bar {bar}%)")
    fw = get_json(base + "/api/firmware")
    st = get_json(base + "/api/status")
    check(fw["version"] == OTA_VERSION and st["version"] == OTA_VERSION and fw["pending"] is None
          and (fw["last"] or "").startswith("updater +") and f"updated to {OTA_VERSION}" in fw["last"],
          f"firmware: the mock runs {OTA_VERSION} ({fw})")
    page.wait_for_function("v => document.querySelector('#fw-facts').textContent.includes('updated to ' + v)"
                           " && document.querySelector('#st-about').textContent.includes(v)", arg=OTA_VERSION, timeout=10000)
    check(True, f"firmware: the card shows {OTA_VERSION} and the updater's line, About too")
    check(len(fw["log"].encode()) <= 1024 and "renamed the bundle to /update/done-" in fw["log"]
          and fw["log"].endswith(fw["last"]), f"firmware: GET /api/firmware's log is the log's end ({fw['log']!r})")
    page.click("#fw-log summary")
    log_text = page.inner_text("#fw-log-text")
    check(page.is_visible("#fw-log-text") and f"renamed the bundle to /update/done-{OTA_VERSION}.bin" in log_text,
          f"firmware: 'Update log' opens on the log ({log_text[-120:]!r})")
    page.locator("#st-fw").screenshot(path=str(OUT / "settings-firmware-laptop.png"))
    expected.remove("/api/firmware")


# js/update.js `wver(s)`: a web app version -> [release, development build].
WVER_CASES = [("v1.0.0", ["1.0.0", False]), ("v1.0.0-3-gabc1234-dirty", ["1.0.0", True]),
              ("v1.0.0-3-gabc1234", ["1.0.0", True]), ("v1.0.0-dirty", ["1.0.0", True]),
              ("v1.0.0-rc.1", ["1.0.0-rc.1", False]), ("v1.0.0-rc.1-2-gabcdef0", ["1.0.0-rc.1", True]),
              ("bb6a3eb", [None, True]), (None, [None, True])]


def release_site(root: Path):
    """The project site's /install/, faked: static files with
    `Access-Control-Allow-Origin: *` as the site sends it, except under
    /nocors/. Returns the server and the paths it was asked for."""
    asked: list[str] = []

    class Site(http.server.SimpleHTTPRequestHandler):
        def __init__(self, *a, **kw):
            super().__init__(*a, directory=str(root), **kw)

        def end_headers(self):
            asked.append(self.path)
            if not self.path.startswith("/nocors/"):
                self.send_header("Access-Control-Allow-Origin", "*")
            super().end_headers()

        def log_message(self, *_a):
            pass
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Site)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv, asked


def update_checks(page: Page, base: str, ota: dict[str, Path]) -> None:
    """Settings -> Advanced -> Firmware -> Check for updates (js/update.js)
    against a locally served release.json and files, installed through the
    mock (its simulated restart, and PUT /api/www adopting the pack's
    version)."""
    tmp = Path(tempfile.mkdtemp(prefix="web-smoke-site-"))
    inst = tmp / "install"
    (inst / "firmware").mkdir(parents=True)
    (tmp / "nocors").symlink_to(tmp)
    shutil.copy(ota["newer"], inst / "firmware" / f"opendmd-{UPD_VERSION}.odfw")
    shutil.copy(ota["newer8"], inst / "firmware" / f"opendmd-{UPD_VERSION}-8mb.odfw")
    shutil.copy(ota["newer_bad"], inst / "firmware" / "tampered.odfw")
    built = (REPO / "opendmd-web" / "www.pack").read_bytes()
    pack = bytearray(built)
    wv = f"v{UPD_VERSION}".encode()
    pack[18] = len(wv)
    pack[20:64] = wv.ljust(44, b"\0")
    (inst / "firmware" / f"opendmd-{UPD_VERSION}-www.pack").write_bytes(bytes(pack))
    fw0 = get_json(base + "/api/firmware")["version"]
    w0 = get_json(base + "/api/www")["version"]

    def release(name: str, **kw) -> str:
        # As tools/site.py writes it: `firmware` (the 4mb bundle, for web
        # apps before 8mb) and `variants`, each layout's file.
        fw4 = kw.get("firmware", f"firmware/opendmd-{UPD_VERSION}.odfw")
        obj = {"version": UPD_VERSION, "firmware": fw4,
               "variants": {"4mb": {"firmware": fw4}, "8mb": {"firmware": f"firmware/opendmd-{UPD_VERSION}-8mb.odfw"}},
               "www": f"firmware/opendmd-{UPD_VERSION}-www.pack", "www_version": f"v{UPD_VERSION}", **kw}
        (inst / name).write_text(json.dumps(obj))
        return f"{site}/install/{name}"
    srv, asked = release_site(tmp)
    site = f"http://127.0.0.1:{srv.server_port}"
    (inst / "garbage.json").write_text("<html>not json</html>")
    # 404s, the CORS refusal, the 422, the restart's 503s: on purpose.
    expected.extend([site, "/api/firmware", "/api/status"])

    def check_now(url: str | None = None) -> str:
        if url is not None:
            page.evaluate("u => localStorage.setItem('odmd.updates', JSON.stringify(u))", url)
        page.evaluate("document.querySelector('#fw-msg').textContent = ''")
        page.click("#fw-check")
        page.wait_for_function("() => { const t = document.querySelector('#fw-msg').textContent;"
                               " return t && !t.startsWith('Checking') && !document.querySelector('#fw-check').disabled; }",
                               timeout=20000)
        return page.inner_text("#fw-msg")
    upd = lambda: page.locator("#fw-upd").count()  # noqa: E731
    try:
        got = page.evaluate("async (cases) => { const { wver } = await import('/js/update.js');"
                            " return cases.map((c) => wver(c)); }", [c for c, _ in WVER_CASES])
        wrong = [(c, g) for (c, w), g in zip(WVER_CASES, got) if g != w]
        check(not wrong, f"update: wver() on {len(WVER_CASES)} web app versions (wrong: {wrong})")
        # ?updates= in the address: this player's own versions -> up to date.
        page.goto(base + "/?updates=" + urllib.parse.quote(release("current.json", version=fw0, www_version=w0)) + "#settings/device")
        goto_view(page, "settings/device")
        page.wait_for_selector("#fw-check", timeout=20000)
        dev = ", development build" if w0 and ("-g" in w0 or w0.endswith("-dirty")) else ""
        t = check_now()
        check(t == f"Up to date ({fw0}{dev})" and not upd(), f"update: ?updates= with the running versions -> '{t}'")
        page.goto(base + "/#settings/device")
        goto_view(page, "settings/device")
        page.wait_for_selector("#fw-check", timeout=20000)
        t = check_now(f"{site}/install/missing.json")
        check("(404)" in t and "bad" in page.get_attribute("#fw-msg", "class") and not upd(), f"update: no release.json -> '{t}'")
        t = check_now(f"{site}/nocors/install/{Path(release('release.json')).name}")
        check(t.startswith(f"Couldn’t reach 127.0.0.1:{srv.server_port}") and not upd(), f"update: no CORS header -> '{t}'")
        t = check_now(f"{site}/install/garbage.json")
        check("not readable" in t and not upd(), f"update: a release.json that is not JSON -> '{t}'")
        t = check_now(release("old.json", version="0.0.1"))
        check(t == f"This player’s firmware ({fw0}) is newer than the latest release (0.0.1)" and not upd(),
              f"update: firmware ahead of the release -> '{t}' (no web app offered either)")
        # A file the release names is missing: nothing installed.
        t = check_now(release("nofile.json", firmware="firmware/nope.odfw"))
        check(t.startswith(f"Firmware {UPD_VERSION} is out (this player has {fw0}).") and upd() == 1, f"update: offered -> '{t}'")
        page.click("#fw-upd")
        page.wait_for_function("() => /404/.test(document.querySelector('#fw-msg').textContent) && !document.querySelector('#fw-check').disabled",
                               timeout=20000)
        t = page.inner_text("#fw-msg")
        check(t == f"The firmware isn’t on 127.0.0.1:{srv.server_port} (404)" and get_json(base + "/api/firmware")["version"] == fw0
              and get_json(base + "/api/www")["version"] == w0, f"update: a missing download installs nothing -> '{t}'")
        # The release's file for the other layout (an 8mb bundle named as the
        # 4mb one): its header gives it away, nothing is sent.
        check_now(release("crossed.json", variants={"4mb": {"firmware": f"firmware/opendmd-{UPD_VERSION}-8mb.odfw"}}))
        page.click("#fw-upd")
        page.wait_for_function("() => /not the release/.test(document.querySelector('#fw-msg').textContent) && !document.querySelector('#fw-check').disabled",
                               timeout=20000)
        t = page.inner_text("#fw-msg")
        check(t == "The firmware download is not the release it should be. Nothing was installed"
              and get_json(base + "/api/firmware")["version"] == fw0 and f"/install/firmware/opendmd-{UPD_VERSION}-8mb.odfw" in asked,
              f"update: the 8mb file named for a 4mb player installs nothing -> '{t}'")
        # The player refuses the firmware (422): no web app sent after it.
        # (`variants` is what a player reads; `firmware` is the good file.)
        check_now(release("tampered.json", variants={"4mb": {"firmware": "firmware/tampered.odfw"}}))
        page.click("#fw-upd")
        page.wait_for_function("() => /SHA-256/.test(document.querySelector('#fw-msg').textContent) && !document.querySelector('#fw-check').disabled",
                               timeout=30000)
        t = page.inner_text("#fw-msg")
        check(t.startswith("The player refused this file:") and get_json(base + "/api/firmware")["version"] == fw0
              and get_json(base + "/api/www")["version"] == w0 and "/install/firmware/tampered.odfw" in asked,
              f"update: a refused firmware stops there, the web app untouched -> '{t}'")
        # The real thing: both, then the reload.
        t = check_now(release("release.json"))
        want = f"Firmware {UPD_VERSION} is out (this player has {fw0}). Web app {UPD_VERSION} is out"
        check(t.startswith(want) and "The panel goes dark" in t and upd() == 1, f"update: both offered -> '{t}'")
        page.locator("#st-fw").screenshot(path=str(OUT / "settings-update-laptop.png"))
        page.evaluate("""() => { sessionStorage.setItem('upd', '[]');
          new MutationObserver(() => { const l = JSON.parse(sessionStorage.getItem('upd'));
            l.push(document.querySelector('#fw-msg').textContent); sessionStorage.setItem('upd', JSON.stringify(l)); })
          .observe(document.querySelector('#fw-msg'), { subtree: true, childList: true, characterData: true }); }""")
        with page.expect_navigation(timeout=90000):
            page.click("#fw-upd")
        page.wait_for_load_state()
        log = json.loads(page.evaluate("sessionStorage.getItem('upd')") or "[]")
        steps = ["Downloading the firmware", "Downloading the web app", "Sending the firmware to the player",
                 "Restarting", "Sending the web app to the player", "Updated. Reloading the page"]
        seen = [any(m.startswith(st) for m in log) for st in steps]
        check(all(seen) and "Sending the firmware to the player… 100%" in log,
              f"update: each step shown ({[st for st, ok in zip(steps, seen) if not ok]} missing; {len(log)} messages)")
        fw, w = get_json(base + "/api/firmware"), get_json(base + "/api/www")
        check(fw["version"] == UPD_VERSION and w["version"] == f"v{UPD_VERSION}" and w["ok"],
              f"update: the mock runs firmware {fw['version']}, web app {w['version']}, and the page reloaded")
        goto_view(page, "settings/device")
        page.wait_for_selector("#fw-check", timeout=20000)
        t = check_now()
        check(t == f"Up to date ({UPD_VERSION})" and not upd(), f"update: after the update -> '{t}'")
    finally:
        for e in (site, "/api/firmware", "/api/status"):
            expected.remove(e)
        page.evaluate("localStorage.removeItem('odmd.updates')")
        put_www(base, built)  # the mock's web app version back to this build's
        srv.shutdown()
        shutil.rmtree(tmp, ignore_errors=True)


def json_type_checks(base: str) -> None:
    """docs/api.md "Ground rules": a JSON body needs its content type."""
    def req(method, path, body, ctype):
        return status_of(urllib.request.Request(base + path, data=body, method=method,
                                                headers={"Content-Type": ctype} if ctype else {}))[0]
    j = b'{"path":"/gifs/Consoles/no-such-file.gif"}'
    for method, path in (("POST", "/api/play"), ("PUT", "/api/config"), ("PUT", "/api/schedule"), ("POST", "/api/wake"),
                         ("POST", "/api/cache/forget"), ("POST", "/api/cache/failed"), ("POST", "/api/follow/test"),
                         ("POST", "/api/files/move")):
        codes = (req(method, path, j, None), req(method, path, j, "text/plain"),
                 req(method, path, j, "application/x-www-form-urlencoded"))
        check(codes == (415, 415, 415), f"content type: {method} {path} without application/json -> {codes}")
    check(req("PUT", "/api/config", b"{}", "Application/JSON; charset=utf-8") == 200,
          "content type: 'Application/JSON; charset=utf-8' is JSON")
    check(req("POST", "/api/skip", b"", None) == 200, "content type: an empty body needs none")
    check(req("PUT", "/api/files/sets/cttest.txt", b"/gifs/x.gif\n", "text/plain") == 200
          and req("DELETE", "/api/files/sets/cttest.txt", None, None) == 200, "content type: raw uploads take any")


EVIL = """<!doctype html>
<iframe name=sink></iframe>
<form id=f method=post action="{base}/api/play" target=sink><input name=path value="{victim}"></form>
<script>fetch('{base}/api/skip', {{method: 'POST', mode: 'no-cors'}}).finally(() => document.getElementById('f').submit());</script>"""


def origin_checks(browser, base: str) -> None:
    """docs/api.md "Security": a page from elsewhere cannot drive the player
    through the browser; scripts (no Origin) and the app (its own) can."""
    host = urllib.parse.urlsplit(base).netloc
    if not get_json(base + "/api/status")["paused"]:
        post_json(base + "/api/pause", {})  # the mock advances by itself otherwise
    before = get_json(base + "/api/status")["now_playing"]
    victim = next(f"/gifs/Consoles/{f['name']}" for f in get_json(base + "/api/files?dir=/gifs/Consoles")["entries"]
                  if not f["dir"] and f["name"].endswith(".gif") and f"/gifs/Consoles/{f['name']}" != before["path"])
    ctx = browser.new_context()
    page = ctx.new_page()
    seen: dict[str, int] = {}
    page.on("response", lambda r: r.url.startswith(base + "/api/") and seen.__setitem__(r.url[len(base):], r.status))
    # Another origin: a page served from another port on this machine.
    # (Chromium's local network checks keep a routed or made-up host from
    # reaching 127.0.0.1 at all, which would prove nothing about the mock.)
    html = EVIL.format(base=base, victim=victim).encode()

    class Evil(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            self.send_response(200)
            self.send_header("Content-Type", "text/html")
            self.send_header("Content-Length", str(len(html)))
            self.end_headers()
            self.wfile.write(html)

        def log_message(self, *_a):
            pass
    evil = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Evil)
    threading.Thread(target=evil.serve_forever, daemon=True).start()
    try:
        page.goto(f"http://127.0.0.1:{evil.server_port}/evil.html")
        deadline = time.time() + 10
        while len(seen) < 2 and time.time() < deadline:
            page.wait_for_timeout(100)
    finally:
        ctx.close()
        evil.shutdown()
    after = get_json(base + "/api/status")["now_playing"]
    check(seen == {"/api/skip": 403, "/api/play": 403}, f"origin: another site's no-cors fetch and form post -> {seen}")
    check(after["ordinal"] == before["ordinal"] and after["path"] == before["path"],
          f"origin: nothing skipped or played (#{before['ordinal']} -> #{after['ordinal']}, {after['path']})")
    post_json(base + "/api/pause", {})

    def put_cfg(origin: str, host_hdr: str | None = None) -> int:
        hd = {"Content-Type": "application/json", "Origin": origin}
        if host_hdr:
            hd["Host"] = host_hdr
        return status_of(urllib.request.Request(base + "/api/config", data=b"{}", method="PUT", headers=hd))[0]
    code, _ = status_of(urllib.request.Request(base + "/api/skip", data=b"", method="POST", headers={"Origin": "http://evil.test"}))
    check(code == 403, f"origin: a script sending Origin http://evil.test -> {code}")
    port = host.split(":")[1]
    codes = (put_cfg(f"http://{host}"), put_cfg(f"HTTP://{host.upper()}"), put_cfg("http://opendmd.local", "opendmd.local:80"),
             put_cfg("http://opendmd.local:8080", "opendmd.local"), put_cfg(f"http://localhost:{port}"))
    check(codes == (200, 200, 200, 403, 403), f"origin: its own (any case, :80 dropped) allowed, others not {codes}")


def key_checks(browser, page: Page, base: str) -> None:
    """The optional API key: Settings -> Advanced -> Security, the 401s, the
    key dialog in a fresh browser, and off again. Last: nothing after it may
    need a key."""
    check(put_status(base, {"api_key": "short"}) == 400 and put_status(base, {"api_key": "has a space"}) == 400,
          "key: a bad api_key is a 400")
    cfg = get_json(base + "/api/config")
    check(cfg.get("api_key_set") is False and "api_key" not in cfg, "key: config has api_key_set false, never the key")
    goto_view(page, "settings/device")
    page.wait_for_selector("#sec-key", timeout=20000)
    check(not page.is_checked("#sec-key") and "plain HTTP" in page.inner_text("#st-sec"), "key: Security card, switch off")
    expected.append("/api/")  # 401s are the point from here on
    try:
        page.check("#sec-key")
        page.wait_for_selector("#sec-key-val", timeout=10000)
        key = page.input_value("#sec-key-val")
        check(re.fullmatch(r"[A-Za-z0-9]{24}", key) is not None and "It is not shown again" in page.inner_text("#st-sec"),
              f"key: a 24-character key, shown once ({key})")
        page.locator("#st-sec").screenshot(path=str(OUT / "settings-security-laptop.png"))
        check(json.loads(page.evaluate("localStorage.getItem('odmd.apiKey')")) == key, "key: kept in this browser")
        code, hd = status_of(urllib.request.Request(base + "/api/status"))
        check(code == 401 and hd.get("WWW-Authenticate") == "Bearer",
              f"key: /api/status without it -> {code}, WWW-Authenticate {hd.get('WWW-Authenticate')}")
        gif = card_gifs()[0]
        gif_url = base + urllib.parse.quote(gif)
        codes = [status_of(urllib.request.Request(u))[0] for u in (gif_url, base + "/filelist.txt", base + "/sets/x.txt",
                                                                  base + "/cache/x.gif")]
        check(codes == [401] * 4, f"key: card files without it -> {codes}")
        k = {"Authorization": "Bearer " + key}
        codes = [status_of(urllib.request.Request(base + "/api/config", headers=k))[0],
                 status_of(urllib.request.Request(base + "/api/status?key=" + key))[0],
                 status_of(urllib.request.Request(gif_url, headers=k))[0],
                 status_of(urllib.request.Request(gif_url + "?key=" + key))[0],
                 status_of(urllib.request.Request(base + "/api/status", headers={"Authorization": "Bearer wrong-wrong"}))[0]]
        check(codes == [200, 200, 200, 200, 401], f"key: the header or ?key= -> 200, a wrong one 401 {codes}")
        codes = [status_of(urllib.request.Request(base + u))[0] for u in ("/", "/index.html", "/app.js", "/js/source.js", "/licenses.txt")]
        check(codes == [200] * 5, f"key: the app's own files need none {codes}")
        check(page.locator("#st-sec .tick.bad").count() == 0 and page.is_checked("#sec-key"), "key: the switch is on, saved")
        # Shown once: a reload (the key is kept here, so no dialog) has the
        # switch on and no key in sight.
        page.reload()
        goto_view(page, "settings/device")
        page.wait_for_function("document.querySelector('#sec-key') && document.querySelector('#sec-key').checked", timeout=20000)
        check(page.locator("#sec-key-val").count() == 0 and page.locator("#key-dlg").count() == 0,
              "key: after a reload the switch is on, the key not shown, no dialog")
        # Another browser: the dialog; a wrong key asks again; the right one works.
        fresh: list[str] = []
        other = new_page(browser, "laptop", fresh)
        other.goto(base + "/")
        other.wait_for_selector("#key-dlg[open]", timeout=20000)
        check("This player needs its API key" in other.inner_text("#key-dlg"), "key: a fresh browser gets the key dialog")
        other.screenshot(path=str(OUT / "key-dialog-laptop.png"))
        other.fill("#key-in", "wrong-key-wrong")
        other.click("#key-dlg button.pri")
        other.wait_for_selector("#key-dlg[open]", timeout=10000)
        check(True, "key: a wrong key asks again")
        other.fill("#key-in", key)
        other.click("#key-dlg button.pri")
        other.wait_for_function("document.querySelector('#now-path') && document.querySelector('#now-path').textContent.startsWith('/')",
                                timeout=20000)
        wait_lit(other, "#now-panel", "key: now")
        check(other.locator("#key-dlg").count() == 0 and json.loads(other.evaluate("localStorage.getItem('odmd.apiKey')")) == key,
              "key: the right key loads Now (preview lit) and is kept")
        other.context.close()
        bad = [e for e in fresh if "HTTP 401" not in e and "401 (Unauthorized)" not in e]
        check(not bad, f"key: the fresh browser's only errors are its 401s {bad[:3]}")
        # Off again: the API is open, the key forgotten here.
        page.uncheck("#sec-key")
        deadline = time.time() + 10
        while status_of(urllib.request.Request(base + "/api/config"))[0] != 200 and time.time() < deadline:
            time.sleep(0.2)
        page.wait_for_function("!JSON.parse(localStorage.getItem('odmd.apiKey') || 'null')", timeout=10000)
        check(get_json(base + "/api/config")["api_key_set"] is False and page.locator("#sec-key-val").count() == 0,
              "key: switched off -> the API is open, the key forgotten here")
    finally:
        expected.remove("/api/")


def mock_pass(browser) -> None:
    overlay = OUT / "overlay"
    shutil.rmtree(overlay, ignore_errors=True)
    overlay.mkdir(parents=True)
    ota_tmp = tempfile.TemporaryDirectory(prefix="web-smoke-ota-")
    ota = ota_bundles(Path(ota_tmp.name))
    proc, base = start_mock(["--root", str(CARD), "--overlay", str(overlay), "--dwell", "4", "--ota-pub", str(ota["pub"]),
                             "--panel-trial", str(PANEL_TRIAL_S)])
    errors: list[str] = []
    try:
        print(f"\n== mock device at {base} (overlay {overlay})")
        # Preparing only while the panel is dark or paused is the default
        # (as on the device); the cache checks rely on it.
        check(get_json(base + "/api/config")["auto_prepare"] == "idle", "auto: auto_prepare defaults to idle")
        page = new_page(browser, "laptop", errors)
        page.goto(base + "/")
        # Now
        page.wait_for_function("document.querySelector('#now-path') && document.querySelector('#now-path').textContent.startsWith('/')", timeout=20000)
        order = page.eval_on_selector_all("#view-now .transport > button:not([hidden])", "els => els.map(e => e.id)")
        check(order[:2] == ["now-skip", "now-pause"] and "pri" in page.get_attribute("#now-skip", "class"),
              f"now: the transport starts with Skip, the primary ({order})")
        lines = page.eval_on_selector_all("#view-now .nowt > div:not([hidden])", "els => els.map(e => e.textContent.trim())")
        check(len(lines) == 4 and re.match(r"^\d+ of [\d,]+$", lines[2]) and lines[3] == "✎ Edit in Create Art"
              and page.locator("#now-more, #now-stats, #now-follow").count() == 0,
              f"now: title, tags, position and Edit on lines of their own; no device stats or Follow switch here ({lines})")
        # The device's figures are Settings › Device's Status. Late frames in
        # plain words (5 ms or more, during play / at GIF changes), every late
        # frame of any size on a line of its own.
        goto_view(page, "settings/device")
        page.wait_for_function("/^\\d+ during play · \\d+ at GIF changes$/.test(document.querySelector('#st-late')?.textContent || '')", timeout=10000)
        m = get_json(base + "/api/status")["misses"]
        late_b = page.locator("#st-late").text_content()
        check(all(k in m for k in ("steady", "switch", "threshold_ms", "window_ms", "worst_steady_ms", "by_cause", "late_ms")),
              "now: status misses carry steady/switch/threshold_ms/window_ms beside by_cause/late_ms")
        check("Late frames, any size" in page.locator("#st-facts").text_content() and "Deadline" not in page.locator("#st-facts").text_content()
              and "5 ms" in (page.locator("#st-late").get_attribute("title") or "") and page.locator("#st-facts dt").count() >= 7,
              f"settings: Device's status reads late frames '{late_b}', the raw count beside it, the threshold in the tooltip")
        goto_view(page, "now")
        check(get_json(base + "/api/status")["manifest"] == "/sdcard/filelist.txt", "now: the default playlist is filelist.txt")
        wait_lit(page, "#now-panel", "now")
        # In step with the mock's clock: a first sync, then two polls.
        st = wait_sync(page, 3)
        check(abs(float(st.get("drift", 1e9))) < 60, f"now: preview in step after two polls (drift {st.get('drift')} ms, rtt {st.get('rtt')} ms, {st.get('path')})")
        check(page.eval_on_selector("#now-cap", "e => e.textContent") == "", "now: no caption once in step")
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
        panel_checks(page, base)
        live_checks(page, base, True)
        no_effects_checks(page, base, True)
        sim_checks(page)
        post_json(base + "/api/skip", {})
        before = page.inner_text("#now-path")
        page.click("#now-skip")
        page.wait_for_function(f"document.querySelector('#now-path').textContent !== {json.dumps(before)}", timeout=10000)
        check(True, "now: skip advanced the device")
        page.click("#now-pause")
        page.wait_for_function("document.querySelector('#now-pause').textContent.startsWith('Resume')")
        check(get_json(base + "/api/status")["paused"], "now: pause reached the device")
        now_fav_checks(page, base)
        page.click("#now-pause")
        page.wait_for_function("document.querySelector('#now-pause').textContent.startsWith('Pause')")
        recent_checks(page, base)
        now_edit_checks(page, base)
        # Send to panel (the Library's button calls app.play; Now has no
        # path box): a real GIF, then a miss (404) and a manifest (415).
        check(page.locator("#now-play-path, #now-play").count() == 0 and page.is_hidden("#now-path"),
              "now: no card path shown, no path box")
        play = "p => import('./js/state.js').then(m => m.app.play(p))"
        gif = card_gifs()[0]
        page.evaluate(play, gif)
        cur = wait_playing(base, gif)
        check(cur == gif, f"now: Send to panel {gif} -> /api/status (got {cur})")
        page.wait_for_function(f"document.querySelector('#now-path').textContent === {json.dumps(gif)}", timeout=10000)
        expected.append("/api/play")
        page.evaluate(play, "/gifs/Consoles/no-such-file.gif")
        t = last_toast(page, "bad")
        check("not on the card" in t, f"now: 404 play -> toast '{t}'")
        page.evaluate("document.querySelectorAll('#toasts .toast').forEach(t => t.remove())")
        page.evaluate(play, "/filelist.txt")
        t = last_toast(page, "bad")
        check("not a GIF" in t and not LEAK.search(t), f"now: 415 play -> toast '{t}'")
        page.evaluate("document.querySelectorAll('#toasts .toast').forEach(t => t.remove())")
        no_leaks(page, "now")
        shot(page, "now", "laptop")
        expected.remove("/api/play")
        # Library, and three Mario GIFs for the cache test
        open_library(page)
        send_from_library(page, base, "library")
        check(page.locator("#lib-list .vrow:not([hidden]) input[type=checkbox]").count() == 0 and page.is_hidden("#lib-selbar"),
              "library: no checkboxes until Select")
        page.click("#lib-select")
        page.wait_for_selector("#lib-selbar:not([hidden])")
        check(page.is_disabled("#lib-to-set") and page.locator("#lib-list .vrow:not([hidden]) input[type=checkbox]").count() > 0,
              "library: Select shows the checkboxes and the bar, Add to playlist disabled with none picked")
        page.click("#lib-list .vrow:not([hidden]) >> nth=0")
        page.eval_on_selector_all("#lib-list .vrow:not([hidden]) input[type=checkbox]",
                                  "els => els.slice(1, 3).forEach(e => e.click())")
        check(page.inner_text("#lib-selbar b") == "3",
              "library: in Select mode a row tap picks it (no preview); three selected")
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
        check(page.locator("#ov-menu #ov-2x, #ov-menu #ov-secs").count() == 0, "row menu: Play longer's fields are a second step")
        page.click("#ov-longer")
        page.click("#ov-2x")
        page.wait_for_selector("#ov-menu", state="detached")
        open_row_menu(page, "#lib-list", 6, via="more")
        page.click("#ov-longer")
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
        tree_checks(browser, base, errors)
        listing_queue_checks(browser, base, errors)
        thumbs_checks(browser, base, errors)
        card_swap_checks(browser, base, errors)
        editor_checks(browser, base, overlay, errors)
        # Sets: opening the view reads no root playlist (nor /gifs) until
        # "Build from filters" is opened.
        fresh = new_page(browser, "laptop", errors)
        reqs: list[str] = []
        fresh.on("request", lambda r: reqs.append(urllib.parse.urlparse(r.url).path))
        fresh.goto(base + "/#sets")
        fresh.wait_for_selector("#view-sets.on")
        fresh.wait_for_selector("#sets-list [data-path='/filelist.xmas']", timeout=20000)
        fresh.wait_for_timeout(1500)
        bad = [r for r in reqs if r.startswith("/gifs") or re.fullmatch(r"/filelist\.[a-z.]+", r)]
        check(not bad and fresh.locator("#builder").count() == 0 and fresh.is_hidden("#set-editor"),
              f"sets: no editor until one is opened, no root playlist or /gifs request until Add by filter opens ({bad})")
        fresh.click("#sets-new")
        fresh.click("#set-build")
        fresh.wait_for_function("document.querySelectorAll('#b-plats .chip').length > 0", timeout=20000)
        check(any(r == "/filelist.txt" for r in reqs), "sets: opening Add by filter reads the base playlist")
        fresh.context.close()
        # Sets: the playlists by name, the default, activation by value
        goto_view(page, "sets")
        page.wait_for_selector("#sets-list [data-path='/filelist.xmas']", timeout=20000)
        rows = root_manifest_rows(page)
        check({"/filelist.txt", "/filelist.hall", "/filelist.xmas"} <= set(rows) and not any("odm" in r for r in rows),
              f"sets: root playlists listed, the obsolete ones not ({rows})")
        labels = {p: row_label(page, p) for p in ["/filelist.txt", "/filelist.hall", "/filelist.xmas"]}
        check(labels == {"/filelist.txt": "All files", "/filelist.hall": "Halloween", "/filelist.xmas": "Christmas"},
              f"sets: friendly labels {labels}")
        dflt = page.eval_on_selector_all("#sets-list [data-path] .tag:not(.act)", "els => els.filter(e => e.textContent === 'Default').map(e => e.closest('[data-path]').dataset.path)")
        check(dflt == ["/filelist.txt"], f"sets: default marked on {dflt}")
        playing = page.eval_on_selector("#sets-list .tag.act", "e => e.closest('button.pl').textContent")
        check("All files" in playing, f"sets: playing '{playing}'")
        sets_do(page, '/filelist.xmas', "activate")
        page.wait_for_function(f"{PLAYING}.includes('Christmas')", timeout=60000)
        st, cf = get_json(base + "/api/status"), get_json(base + "/api/config")
        check(cf["set"] == "filelist.xmas" and st["manifest"] == "/sdcard/filelist.xmas",
              f"sets: Christmas activated as set \"filelist.xmas\" ({st['entries']} entries)")
        no_leaks(page, "sets")
        shot(page, "sets-root", "laptop")
        sets_do(page, '/filelist.txt', "activate")
        page.wait_for_function(f"{PLAYING}.includes('All files')", timeout=60000)
        st, cf = get_json(base + "/api/status"), get_json(base + "/api/config")
        check(cf["set"] == "" and st["manifest"] == "/sdcard/filelist.txt", "sets: All files activated as set \"\"")
        n = build_smoke_set(page)
        written = overlay / "sets" / "smoke.txt"
        want = expected_md_snes()
        got = written.read_text().split("\n")[:-1] if written.exists() else []
        check(written.exists() and got == want, f"sets: sets/smoke.txt holds the {len(want)} MD/SNES lines in order (got {len(got)}, editor {n})")
        # Replace on a saved playlist keeps it that playlist: same header, saved in place.
        page.click("#set-build")
        page.wait_for_selector("#builder #b-plats [data-platform=SNES].on", timeout=10000)
        page.click("#b-plats [data-platform=SNES]")
        page.wait_for_function("/^Replace with [1-9]/.test(document.querySelector('#b-use').textContent)", timeout=10000)
        page.click("#b-use")
        page.wait_for_selector("#builder", state="detached", timeout=5000)
        check(page.inner_text("#set-title") == "smoke" and page.is_hidden("#set-name-row"),
              f"sets: Replace keeps the playlist open as itself ({page.inner_text('#set-title')!r})")
        page.click("#set-save")
        page.wait_for_function(f"document.querySelector('#set-count').textContent.split(' ')[0] !== {json.dumps(str(n))}", timeout=10000)
        deadline = time.time() + 10
        while time.time() < deadline and len(written.read_text().split("\n")[:-1]) == len(want):
            time.sleep(0.2)
        md = written.read_text().split("\n")[:-1]
        check(0 < len(md) < len(want) and set(md) <= set(want), f"sets: saved in place with the new contents ({len(md)} of {len(want)} lines)")
        written.write_text("\n".join(want) + "\n")  # as it was, for what follows
        sets_do(page, '/sets/smoke.txt', "activate")
        page.wait_for_function(f"{PLAYING}.includes('smoke')", timeout=60000)
        st = get_json(base + "/api/status")
        check(get_json(base + "/api/config")["set"] == "smoke" and st["manifest"] == "/sdcard/sets/smoke.txt",
              f"sets: activated ({st['entries']} entries)")
        shot(page, "sets", "laptop")
        # A loaded device: the first listing of /sets after this is delivered
        # 1.2 s late (read when asked, so it still holds the file) and the
        # DELETE reaches the device late. The config change that Delete starts
        # (back to All files) lists the card before the file is gone; that
        # listing, landing after the one the delete started, must not be what
        # the page shows.
        page.evaluate("""() => {
            const f = window.fetch.bind(window);
            window.__fetch = f; window.__slow = 0;
            window.fetch = async (u, ...a) => {
                if (a[0]?.method === 'DELETE') await new Promise((k) => setTimeout(k, 150));
                const r = await f(u, ...a);
                if (String(u).includes('/api/files') && /dir=(%2F|\\/)sets/.test(String(u))) await new Promise((k) => setTimeout(k, window.__slow++ ? 0 : 1200));
                return r;
            };
        }""")
        sets_do(page, '/sets/smoke.txt', "delete")
        page.wait_for_selector("#sets-list [data-path='/sets/smoke.txt']", state="detached", timeout=10000)
        page.wait_for_timeout(1500)  # past any late listing
        check(page.locator("#sets-list [data-path='/sets/smoke.txt']").count() == 0,
              "sets: the deleted playlist stays gone when an earlier listing lands late")
        page.evaluate("() => { window.fetch = window.__fetch; }")
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
        page.click("#sets-list [data-path='/sets/overridetest.txt'] button.pl")
        # The editor still shows the previous draft (smoke, MD only) until
        # open() has fetched the file: wait for its header and count, which
        # it sets together with the rows, not just for any row.
        page.wait_for_function("document.querySelector('#set-title').textContent === 'overridetest'"
                               f" && document.querySelector('#set-count').textContent.startsWith('{len(ov_set_lines)} GIFs')", timeout=20000)
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
        reorder_checks(page, ov_set_lines)
        no_leaks(page, "playlists (overrides)")
        # Cache: the three selected GIFs as a playlist, their copies forgotten.
        goto_view(page, "library")
        page.click("#lib-to-set")
        page.wait_for_selector("#view-sets.on")
        page.evaluate("async (sel) => { const { app } = await import('./js/state.js'); const { emit } = await import('./js/util.js'); app.draft = { paths: sel, name: '', path: '' }; emit('draft'); }", selected)
        page.fill("#set-name", "cachetest")
        page.click("#set-save-activate")
        page.wait_for_function(f"{PLAYING}.includes('cachetest')", timeout=60000)
        check(get_json(base + "/api/status")["entries"] == 3, "cache: playlist cachetest (the three selected) playing")
        forgot = [post_json(base + "/api/cache/forget", {"path": p})["removed"] for p in selected]
        check(all(forgot), f"cache: POST /api/cache/forget removed the three copies ({forgot})")
        goto_view(page, "settings/device")
        line = wait_cache_line(page, "0 of 3")
        check(True, f"cache: settings card reads '{line}'")
        no_leaks(page, "settings (cache)")
        # Auto prepare: playing with "idle" -> waiting; paused -> the mock's
        # simulated builder runs at full pace (it writes nothing).
        a = auto_fields(base, "auto")
        check(a["mode"] == "idle" and a["state"] == "paused" and a["reason"] == "playing", f"auto: playing -> paused ({a['state']}, {a['reason']})")
        # The ETA follows once the mock's builder has measured its full pace:
        # the earlier pauses (Now's preview, Now's Pause) can add up to a
        # file's 2 s, so it may or may not have by now.
        t = wait_auto_text(page, "^Files are prepared automatically when the panel is idle · 0 of 3( · about \\d+ s of idle time left)?\\.")
        a = get_json(base + "/api/cache")["auto"]
        eta = "of idle time left" in t
        check(not page.is_checked("#pc-always") and eta == (a["avg_full_s"] is not None),
              f"auto: card reads '{t}', option unticked (avg_full_s {a['avg_full_s']})")
        try:
            put_json(base + "/api/config", {"auto_prepare": "sometimes"})
            check(False, "auto: a bad auto_prepare is refused")
        except urllib.error.HTTPError as e:
            check(e.code == 400 and "idle, always or off" in e.read().decode(), f"auto: a bad auto_prepare is a {e.code}")
        post_json(base + "/api/pause", {})
        goto_view(page, "sets")
        goto_view(page, "settings/device")
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
        add_files(page, adds)
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
        # A names list gone stale (the device's names file from before a GIF
        # came): the dialog picks a taken name, the create-only upload gets a
        # 409, and the GIF goes in under the next number; the one there is
        # left as it was.
        taken = "/gifs/Consoles/SNES_Smoke_Stale_01.gif"
        before = (CARD / selected[0].lstrip("/")).read_bytes()
        urllib.request.urlopen(urllib.request.Request(base + "/api/files" + taken, data=before, method="PUT"), timeout=30).read()
        urllib.request.urlopen(base + "/mock/stale-names?hide=" + taken, timeout=10).read()
        stale_src = src_dir / "SNES_Smoke_Stale_01.gif"
        shutil.copy(CARD / selected[1].lstrip("/"), stale_src)
        add_files(page, [stale_src])
        page.wait_for_function("(e => e && e.dataset.name === 'SNES_Smoke_Stale_01.gif')(document.querySelector('#dl .dl-item[data-i=\"0\"]'))", timeout=20000)
        page.click("#dl-ok")
        page.wait_for_function("document.querySelector('#lib-add-progress').textContent.startsWith('1 of 1 added')", timeout=60000)
        second = overlay / "gifs/Consoles/SNES_Smoke_Stale_02.gif"
        check(second.exists() and second.read_bytes() == stale_src.read_bytes() and (overlay / taken.lstrip("/")).read_bytes() == before,
              "add: a name taken since the names list was made (409) -> the GIF goes in as _02, the _01 there untouched")
        for p in (taken, "/gifs/Consoles/SNES_Smoke_Stale_02.gif"):
            urllib.request.urlopen(urllib.request.Request(base + "/api/files" + p, method="DELETE"), timeout=30).read()
        details_checks(page, base, overlay)
        studio_checks(page, base, overlay)
        studio_anim_checks(page, base, overlay)
        studio_smooth_checks(page, base, overlay)
        formulas_checks(browser, base, errors)
        formulas_panel_checks(browser, base, errors)
        formula_keep_checks(browser, base, errors)
        formulas_playlist_checks(browser, base, overlay, errors)
        formulas_remix_checks(browser, base, overlay, errors)
        import_checks(browser, base, overlay, errors)
        formulas_store_checks(browser, base, overlay, errors)
        formulas_over_checks(browser, base, errors)
        looks_checks(browser, base, errors)
        find_checks(page)
        # Back to All files; the test playlist goes.
        goto_view(page, "sets")
        sets_do(page, '/sets/cachetest.txt', "delete")
        page.wait_for_selector("#sets-list [data-path='/sets/cachetest.txt']", state="detached", timeout=60000)
        check(get_json(base + "/api/config")["set"] == "", "cache: cachetest deleted, All files playing")
        smooth_checks(page, base, overlay)
        # Settings
        settings_tab_checks(page)
        goto_view(page, "settings/play")
        # The heading's save count before this change (an earlier save may
        # have shown, and cleared, its tick already).
        n0 = page.evaluate("+(document.querySelector('#st-tab-play .card h2 .tick')?.dataset.n || 0)")
        page.eval_on_selector("#st-brightness", "e => { e.value = 120; e.dispatchEvent(new Event('input')); }")
        check(wait_config(base, lambda c: c["brightness"] == 120)["brightness"] == 120, "settings: brightness saved as it settles, read back")
        page.wait_for_function("n => +document.querySelector('#st-tab-play .card h2 .tick').dataset.n > n", arg=n0, timeout=5000)
        tick = page.text_content("#st-tab-play .card h2 .tick")
        anim = page.eval_on_selector("#st-tab-play .card h2 .tick", "e => getComputedStyle(e).animationName")
        check(tick == "Saved ✓" and anim == "out", f"settings: the Playback heading shows Saved ✓, fading ({tick!r}, {anim})")
        page.wait_for_function("document.querySelector('#st-tab-play .card h2 .tick').textContent === ''", timeout=5000)
        check(True, "settings: the tick goes after a moment")
        goto_view(page, "settings/device")
        page.wait_for_selector("#st-facts dt")
        page.click("#st-scan")
        page.wait_for_selector("#st-nets .chip", timeout=15000)
        check(True, "settings: WiFi scan listed networks")
        hostname_checks(page, base)
        names_checks(page, base)
        platform_checks(page, base)
        version = (REPO / "opendmd-web" / "www" / "version.txt").read_text().strip()
        goto_view(page, "settings/device")
        facts = page.inner_text("#st-facts")
        about = page.inner_text("#st-about")
        check(version in about and "GIFs" in facts and "Late frames" in facts and "index" not in facts.lower(),
              f"settings: www_version {version} (About), the status without 'index'")
        www_mock_checks(page, base, version)
        api_page_checks(page, base)
        goto_view(page, "settings/play")
        opts = page.eval_on_selector_all("#st-set option", "els => els.map(e => [e.value, e.textContent])")
        check(opts[0] == ["", "All files (default)"] and ["filelist.xmas", "Christmas"] in opts
              and not any("odm" in v for v, _ in opts), f"settings: playlist choices {opts[:4]}")
        for t in ("play", "look", "time", "device"):
            goto_view(page, f"settings/{t}")
            no_leaks(page, f"settings ({t})")
            page.evaluate("window.scrollTo(0, 0)")
            shot(page, f"settings-{t}", "laptop")
        goto_view(page, "settings/time")
        # Time: pick a zone, save, and check the POSIX TZ string landed.
        page.select_option("#tm-zone", label="Europe/Dublin")
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
        want = {"from": night_from, "to": night_to, "mode": "off", "brightness": 8}
        sched = wait_schedule(base, lambda s: s.get("night") == want)
        # schedule.js always sends the night brightness slider's value (8 by
        # default) alongside from/to/mode.
        check(sched.get("night") == {"from": night_from, "to": night_to, "mode": "off", "brightness": 8},
              f"settings: night mode saved ({sched.get('night')})")
        no_leaks(page, "settings (schedule)")
        # Phone: the schedule card and the Now banner.
        phone = new_page(browser, "phone", errors)
        phone.goto(base + "/")
        phone.wait_for_function("document.querySelector('#now-path') && document.querySelector('#now-path').textContent.startsWith('/')", timeout=20000)
        goto_view(phone, "settings/time")
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
        goto_view(page, "settings/device")
        # The panel is off for the night: the builder runs, counted in nights.
        t = wait_auto_text(page, "^Preparing files automatically while the panel is idle · [\\d,]+ of [\\d,]+ · about \\d+ nights? left\\.|^All files are prepared\\.")
        check(True, f"auto: night mode -> '{t}'")
        no_leaks(page, "settings (auto)")
        goto_view(page, "settings/time")
        page.click("#sc-rules [data-rule='0'] >> text=Delete")
        page.wait_for_selector("#sc-rules [data-rule]", state="detached", timeout=10000)
        page.uncheck("#sc-night-on")
        sched = wait_schedule(base, lambda s: s.get("night") is None)
        check(sched == {"rules": [], "night": None}, f"settings: schedule cleared -> empty form ({sched})")
        goto_view(page, "now")
        page.wait_for_selector("#now-banner", state="hidden", timeout=15000)
        check(get_json(base + "/api/status")["mode"] == "normal", "now: schedule cleared -> status mode normal")
        goto_view(page, "settings/device")
        # index_state "building": set switching is disabled until it is ready.
        def building(route):
            r = route.fetch()
            route.fulfill(response=r, json={**r.json(), "index_state": "building"})
        page.route("**/api/status", building)
        page.evaluate("async () => { const { app } = await import('./js/state.js'); await app.refreshStatus(); }")
        check("updating" in page.inner_text("#st-facts"), "settings: building -> the status says the playlist is updating")
        goto_view(page, "settings/play")
        page.wait_for_function("document.querySelector('#st-set').disabled", timeout=5000)
        check(True, "settings: building -> set select disabled")
        shot(page, "settings-building", "laptop")
        goto_view(page, "sets")
        page.click("#sets-list [data-path='/filelist.xmas'] button.more")
        check(page.is_disabled("#sets-menu [data-do=activate]"), "sets: building -> Activate disabled")
        shot(page, "sets-building", "laptop")
        page.keyboard.press("Escape")
        page.unroute("**/api/status")
        goto_view(page, "settings")
        page.wait_for_function("!document.querySelector('#st-set').disabled", timeout=8000)
        check(True, "settings: ready again -> set select enabled")
        views = page.eval_on_selector_all("#nav a[data-view]", "els => els.map(e => e.querySelector('span').textContent)")
        check(views == ["Now", "Library", "Playlists", "Formulas", "Create Art", "Settings"] and page.locator("#view-convert").count() == 0,
              f"nav: six views, Playlists third, Formulas before Create Art, no Convert ({views})")
        follow_checks(page, base, errors)
        font_checks(page, base, "laptop")
        phone_pass(browser, base, errors)
        recent_demo_pass(browser, base, errors)
        formulas_phone_pass(browser, base, errors)
        formulas_demo_pass(browser, base, errors)
        json_type_checks(base)
        origin_checks(browser, base)
        firmware_checks(page, base, ota)
        update_checks(page, base, ota)
        key_checks(browser, page, base)
    finally:
        proc.terminate()
        ota_tmp.cleanup()
    report_errors("mock", errors)


# --- The Studio ("Create Art") -------------------------------------------------

ART = REPO / "opendmd-art-web"
STUB_INDEX = {"repo": "Stub_-_Repo", "names": ["Other Game (Europe)", "Studio Test (Japan)", "Studio Test (USA)"],
              "kinds": "123"}


# --- Formulas (Library → Formulas, js/formulas.js + pkg-psi) -------------------

FORMULAS_JSON = REPO / "opendmd-web" / "www" / "formulas.json"


def bundled() -> list[dict]:
    """The bundled Formulas (formulas.json, written by make web from the
    `-- bundle` files: `[{src}]`), each with the name its `name "…"` line gives."""
    return [{**e, "name": re.search(r'^name "([^"]*)"', e["src"], re.M).group(1)} for e in json.loads(FORMULAS_JSON.read_text())]


# An example that draws over a GIF: it reads what is under it, or has layers.
def psi_over(page: Page, srcs: list[str]) -> list[bool]:
    """Which formulas draw over what is under them: info()'s bit 5, asked of
    a fresh instance of pkg-psi (the app's own rule)."""
    return page.evaluate("""async (srcs) => { const m = await import('./pkg-psi/opendmd_psi_web.js?over'); await m.default();
        return srcs.map(s => m.compile(s) && !!(m.info()[5] & 32)); }""", srcs)


def open_formulas(page: Page) -> None:
    goto_view(page, "formulas")
    page.wait_for_selector("#psi-ex .psi-tile", timeout=20000)


def formula_editor(page: Page, src: str) -> None:
    """The editor with `src` typed in (+ New formula, then the text replaced)."""
    open_formulas(page)
    page.click("#psi-new")
    page.wait_for_selector("#psi-ed #psi-src", timeout=10000)
    page.fill("#psi-src", src)
    page.wait_for_timeout(400)  # the 150 ms compile, a frame or two


def remix_open(page: Page, base: str, errors: list[str], src, name="", bad_first=False, **vals) -> Page:
    """The panel's app opened at `#remix` by `page` (as the site's page opens
    it with Open on my panel, here from the app's own origin, which it
    accepts as it does opendmd.dev), its ready answered with the Formula by
    postMessage (`bad_first`: a message of the wrong shape first). Returns
    the app's page, its editor up."""
    page.evaluate("""([src, name, vals, bad]) => addEventListener('message', function on(e) {
        if (e.data?.type !== 'opendmd-remix-ready') return;
        removeEventListener('message', on);
        if (bad) e.source.postMessage({ type: 'opendmd-remix', src: 42 }, '*');
        e.source.postMessage({ type: 'opendmd-remix', name, src, ...vals }, '*');
    })""", [src, name, vals, bad_first])
    with page.context.expect_page() as info:
        page.evaluate("u => { window.open(u, '_blank'); }", base + "/#remix")
    app = info.value
    app.on("pageerror", lambda e: errors.append(f"[remix] pageerror: {e}"))
    app.wait_for_selector("#psi-ed #psi-src", timeout=20000)
    app.wait_for_timeout(500)
    return app


def scrub(page: Page, line: int, col: float, dx: int) -> None:
    """A mouse drag of `dx` px on the text at (line, column), 0-based."""
    box = page.locator("#psi-src").bounding_box()
    cw, lh = page.evaluate("""() => { const ta = document.querySelector('#psi-src'), s = document.createElement('span');
        s.textContent = '0'.repeat(20); document.querySelector('.psi-hl').append(s);
        const w = s.getBoundingClientRect().width / 20; s.remove(); return [w, parseFloat(getComputedStyle(ta).lineHeight)]; }""")
    x, y = box["x"] + 1 + 8 + cw * (col + 0.5), box["y"] + 1 + 8 + lh * (line + 0.5)
    page.mouse.move(x, y)
    page.mouse.down()
    page.mouse.move(x + dx, y, steps=8)
    page.mouse.up()
    page.wait_for_timeout(400)


def psi_memory_checks(page: Page) -> None:
    """The module's bump allocator starts over between calls: thousands of
    compiles (good and bad), errors and frames leave its memory as it was."""
    grown = page.evaluate("""async () => {
        const m = await import('./pkg-psi/opendmd_psi_web.js'), w = await m.default();
        const srcs = ['loop 4\\nv = sin(x*2 + t/4)\\ngray(v)', 'gray(nope + 2x)', 'pal(fire, len(x - 0.5, y - 0.5))'];
        for (let k = 0; k < 60; k++) { m.compile(srcs[k % 3]); m.error(); m.info(); }
        const before = w.memory.buffer.byteLength;
        for (let k = 0; k < 6000; k++) { m.compile(srcs[k % 3]); m.error(); m.errorAt(); m.info(); m.frame(k, 1, 2, 3, 0, 0, 0); }
        return [before, w.memory.buffer.byteLength];
    }""")
    check(grown[0] == grown[1], f"formulas: 6,000 compiles leave the module's memory as it was ({grown})")


def psi_error_links(page: Page) -> None:
    """The editor's error code: the link to the language reference's entry
    on the project site (a new tab), the code following each new error;
    and js/psierr.js's codeOf giving every golden case in
    opendmd-psi/tests/diagnostics.txt the code the compiler's own index
    gives it (`=== E…`), so the JS table and its weighing are the Rust's."""
    a = page.locator("#psi-err a.ecode")
    check(a.inner_text() == "E201" and a.get_attribute("href") == "https://opendmd.dev/dev/psi/#E201"
          and a.get_attribute("target") == "_blank" and "noopener" in (a.get_attribute("rel") or ""),
          f"formulas: the error's code links the reference ({a.get_attribute('href')})")
    page.fill("#psi-src", "gray(sine(x))")
    page.wait_for_function("document.querySelector('#psi-err a.ecode')?.textContent == 'E202'", timeout=10000)
    check(page.locator("#psi-err a.ecode").count() == 1 and page.get_attribute("#psi-err a.ecode", "href").endswith("#E202"),
          "formulas: a new error, its own code (one link)")
    cases = re.findall(r"^=== E(\d+)\n(?:.*\n)*?--- \d+:\d+ (.*)$",
                       (REPO / "opendmd-psi/tests/diagnostics.txt").read_text(encoding="utf-8"), re.M)
    bad = page.evaluate("""async (cases) => { const m = await import('./js/psierr.js');
        return cases.filter(([c, msg]) => m.codeOf(msg) !== +c).slice(0, 5); }""", cases)
    check(len(cases) >= 150 and not bad, f"formulas: psierr.js codes all {len(cases)} golden messages as the compiler's index does ({bad})")


def formulas_checks(browser, base: str, errors: list[str]) -> None:
    """Library → Formulas: the module loads only then; the examples render;
    a plain error with its line; a number scrubbed changes the text, the
    preview and is one undo; a slider for `a`; saving; a Formula sent by postMessage round
    trip; Save as GIF into Create Art with one loop of frames."""
    page = new_page(browser, "laptop", errors)
    asked: list[str] = []
    page.on("request", lambda r: asked.append(r.url))
    page.goto(base + "/#library")
    page.wait_for_selector("#lib-search", timeout=20000)
    goto_view(page, "library")
    page.wait_for_timeout(500)
    early = [u for u in asked if "pkg-psi" in u or "formulas" in u]
    check(not early, f"formulas: nothing of them loads with the app or the Library ({early})")
    # The old address of the editor (Formulas were a Library tab) leads to the view's.
    page.evaluate("location.hash = '#library/formula'")
    page.wait_for_function("location.hash == '#formulas/edit'", timeout=20000)
    open_formulas(page)
    # (Each at its build's address, ?v=<build id>: js/wasm.js.)
    psi = [u for u in asked if "pkg-psi/" in u and urllib.parse.urlsplit(u).path.endswith(".wasm")]
    check(bool(psi) and all(re.search(r"\?v=[0-9a-f]{16}$", u) for u in psi),
          f"formulas: the module's wasm asked for at the build's address ({[u.rsplit('/', 1)[-1] for u in psi]})")
    examples = bundled()
    tiles = page.eval_on_selector_all("#psi-ex .psi-tile", "els => els.map(e => e.dataset.name)")
    more = page.get_attribute("#psi-more", "href")
    dark = page.evaluate("""() => [...document.querySelectorAll('#psi-ex .psi-tile canvas')].filter(c => {
        const d = c.getContext('2d').getImageData(0, 0, 128, 32).data; for (let i = 0; i < d.length; i += 4) if (d[i] + d[i+1] + d[i+2] > 90) return false; return true; }).length""")
    # One card, Bundled: the ten the panel comes with (those that draw over a
    # GIF have the look badge), and More Formulas: the online gallery.
    over = psi_over(page, [e["src"] for e in examples])
    over = [e["name"] for e, o in zip(examples, over) if o]
    badged = page.eval_on_selector_all("#psi-ex .psi-tile:has(.psi-badge)", "els => els.map(e => e.dataset.name)")
    check(len(psi) == 1 and tiles == [e["name"] for e in examples] and len(tiles) == 10 and badged == over and len(over) == 5 and dark == 0
          and page.locator("#psi-ex-over").count() == 0 and more == "https://opendmd.dev/formulas/",
          f"formulas: opening the tab loads the module ({len(psi)}); {len(tiles)} bundled ({len(over)} looks), every thumbnail lit ({dark} dark); More Formulas -> {more}")
    wait_lit(page, "#psi-panel", "formulas: the first example in Details")
    a = canvas_sum(page, "#psi-panel")
    page.wait_for_timeout(300)
    check(canvas_sum(page, "#psi-panel") != a and page.inner_text("#psi-d-title") == examples[0]["name"]
          and page.is_hidden("#psi-del") and page.inner_text("#psi-edit") == "Remix",
          f"formulas: {examples[0]['name']} plays in Details, Remix, no Delete")
    no_leaks(page, "library (formulas)")
    shot(page, "library-formulas", "laptop")
    psi_memory_checks(page)

    # An error: a plain sentence with its line; the last version keeps playing.
    formula_editor(page, "loop 4\nv = sin(x*2 + t/4)\ngray(tme)")
    page.wait_for_selector("#psi-err a.ecode", timeout=10000)
    err = page.inner_text("#psi-err")
    check(err == "Line 3: unknown name tme — did you mean t? E201" and page.get_attribute("#psi-ed", "data-err") == "3"
          and page.locator(".psi-hl .e").inner_text() == "tme" and page.locator(".psi-gut b").inner_text() == "3",
          f"formulas: an error says '{err}' and marks the word and the line")
    shot(page, "formulas-error", "laptop")
    psi_error_links(page)
    # A still formula: a scrubbed number changes the text and the picture.
    page.fill("#psi-src", "gray(x*0.5)")
    page.wait_for_timeout(400)
    wait_lit(page, "#psi-ed-panel", "formulas: the editor's preview")
    before = canvas_sum(page, "#psi-ed-panel")
    scrub(page, 0, 8, 40)
    src = page.input_value("#psi-src")
    page.wait_for_timeout(200)
    m = re.fullmatch(r"gray\(x\*(\d\.\d)\)", src)
    check(m is not None and float(m.group(1)) > 0.5,
          f"formulas: dragging 0.5 right makes it bigger ({src!r})")
    check(canvas_sum(page, "#psi-ed-panel") != before, "formulas: and the preview changes")
    page.keyboard.press("Control+z")
    page.wait_for_timeout(300)
    check(page.input_value("#psi-src") == "gray(x*0.5)", f"formulas: one undo puts the drag back ({page.input_value('#psi-src')!r})")
    # Below zero: a minus of its own, or brackets where it would make `--`.
    scrub(page, 0, 8, -40)
    neg = page.input_value("#psi-src")
    page.fill("#psi-src", "gray(1-0.5)")
    page.wait_for_timeout(300)
    scrub(page, 0, 8, -40)
    neg2 = page.input_value("#psi-src")
    check(neg == "gray(x*-0.5)" and neg2 == "gray(1-(-0.5))" and page.get_attribute("#psi-ed", "data-err") is None,
          f"formulas: a number dragged below zero ({neg!r}, {neg2!r}) still compiles")
    page.fill("#psi-src", "gray(x*0.5)")
    page.wait_for_timeout(300)
    # Alt+↑ on a number (keyboard).
    page.focus("#psi-src")
    page.evaluate("document.querySelector('#psi-src').setSelectionRange(8, 8)")
    page.keyboard.press("Alt+ArrowUp")
    page.wait_for_timeout(200)
    check(page.input_value("#psi-src") == "gray(x*0.6)", f"formulas: Alt+↑ nudges the number ({page.input_value('#psi-src')!r})")
    # A choice: `control name = word, word, …` is a row of buttons (a radio
    # group); the value is the choice's number, and the arrows move it.
    page.fill("#psi-src", "control level = off, half, full\ngray(level/2)")
    page.wait_for_timeout(400)
    grp = page.locator("#psi-ed-sl-a")
    radios = page.eval_on_selector_all("#psi-ed-sl-a [role=radio]", "els => els.map(e => [e.textContent, e.getAttribute('aria-checked')])")
    named = page.inner_text("#psi-ed-sl .psi-sl > span")
    page.click("#psi-ed-sl-a [role=radio] >> text=half")
    page.wait_for_timeout(400)
    draft_a = "import('./js/util.js').then(m => m.store.get('psi.draft').a)"
    mid, a_mid = canvas_sum(page, "#psi-ed-panel"), page.evaluate(draft_a)
    page.keyboard.press("ArrowRight")
    page.wait_for_timeout(400)
    full, a_full = canvas_sum(page, "#psi-ed-panel"), page.evaluate(draft_a)
    focused = page.evaluate("[document.activeElement.textContent, document.activeElement.getAttribute('aria-checked')]")
    page.keyboard.press("Home")
    page.wait_for_timeout(400)
    off, a_off = canvas_sum(page, "#psi-ed-panel"), page.evaluate(draft_a)
    check(grp.get_attribute("role") == "radiogroup" and grp.get_attribute("aria-label") == "level" and named == "level"
          and [r[0] for r in radios] == ["off", "half", "full"] and (a_mid, a_full, a_off) == (1, 2, 0)
          and focused == ["full", "true"] and off < mid < full,
          f"formulas: a choice's buttons ({radios}) set a to 1, 2 (→), 0 (Home) and the preview follows ({off}, {mid}, {full})")
    # A slider, declared by a control line.
    page.fill("#psi-src", "control brightness = 0\ngray(brightness)")
    page.wait_for_timeout(400)
    names = page.eval_on_selector_all("#psi-ed-sl .psi-sl span", "els => els.map(e => e.textContent)")
    page.eval_on_selector("#psi-ed-sl-a", "r => { r.value = 0.75; r.dispatchEvent(new Event('input')); }")
    page.wait_for_timeout(300)
    px = page.evaluate("""() => { const c = document.querySelector('#psi-ed-panel'), d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
        let m = 0; for (let i = 0; i < d.length; i += 4) m = Math.max(m, d[i]); return m; }""")
    check(names == ["brightness"] and px > 120, f"formulas: a control's slider, named '{names}', lights the panel ({px})")
    # Saved, then listed under Your formulas.
    page.fill("#psi-name", "Smoke brightness")
    page.click("#psi-save")
    page.wait_for_selector("#psi-ed .tick.ok", timeout=5000)
    # No links with a Formula in them: Copy code, and Submit to the gallery
    # (the form the site's page has: the file with its header, GitHub's
    # new-file page filled in).
    check(page.locator("#psi-ed-share").count() == 0 and page.inner_text("#psi-ed-code") == "Copy code",
          "formulas: no share link, Copy code instead")
    page.click("#psi-ed-submit")
    page.wait_for_selector("#psi-submit-sheet #s-text", timeout=10000)
    page.fill("#s-desc", "A smoke test")
    page.fill("#s-author", "@smoke")
    page.wait_for_timeout(200)
    text, go = page.inner_text("#s-text"), page.get_attribute("#s-go", "href")
    shot(page, "formulas-submit", "laptop")
    page.keyboard.press("Escape")
    check(text.startswith("-- author: @smoke\n-- licence: CC0-1.0\nname \"Smoke brightness\"\nabout \"A smoke test\"\n")
          and go.startswith("https://github.com/opendmd/opendmd.github.io/new/main/gallery?filename=smoke-brightness.psi&value="),
          f"formulas: Submit to the gallery fills in the file and GitHub's page ({text[:60]!r}, {go[:90]})")
    # From the site (Open on my panel, edited): the panel's app opened by its
    # page gets the Formula by postMessage, slider and all.
    opener = new_page(browser, "laptop", errors)
    opener.goto(base + "/version.txt")
    other = remix_open(opener, base, errors, 'name "Smoke brightness"\ncontrol brightness = 0\ngray(brightness)', "Smoke brightness", a=0.75)
    got = (other.input_value("#psi-src"), other.input_value("#psi-name"), other.input_value("#psi-ed-sl-a"))
    check(got == ('name "Smoke brightness"\ncontrol brightness = 0\ngray(brightness)', "Smoke brightness", "0.75") and other.evaluate("location.hash") == "#formulas/edit",
          f"formulas: a Formula sent from the site's page opens in the editor ({got})")
    opener.context.close()
    page.click("#psi-back")
    page.wait_for_selector("#psi-mine .psi-tile", timeout=5000)
    check(page.eval_on_selector_all("#psi-mine .psi-tile", "els => els.map(e => e.dataset.name)") == ["Smoke brightness"]
          and page.evaluate("location.hash") == "#formulas", "formulas: back to the gallery, saved under Your formulas")
    # Save as GIF: one loop (4 s at 20 ms) into Create Art.
    page.click("#psi-ex .psi-tile >> nth=0")
    formula_editor(page, "loop 4\npal(fire, saw(x - t/4)*0.5 + 0.5)")
    page.fill("#psi-name", "Smoke fire")
    page.click("#psi-ed-gif")
    page.wait_for_selector("#view-create.on #studio .st-grid", timeout=20000)
    page.wait_for_function("(d => d.dataset.animated && d.dataset.srcFrames === '200' && d.dataset.name)(document.querySelector('#studio'))", timeout=30000)
    d = studio_data(page)
    msg = page.inner_text("#st-msg")
    check(d.get("name") == "FX_Smoke_fire_01.gif" and page.input_value("#st-plat") == "FX" and "200 frames" in msg and "one loop" in msg
          and d.get("duration") == "4000", f"formulas: Save as GIF -> Create Art, {d.get('srcFrames')} frames, '{msg}', {d.get('name')}")
    shot(page, "formulas-gif", "laptop")
    page.context.close()
    formulas_more(browser, base, errors)
    formulas_combine_checks(browser, base, errors)


def formulas_playlist_checks(browser, base: str, overlay: Path, errors: list[str]) -> None:
    """Formulas by reference. Saved Formulas live on the panel: those saved
    in this browser before are copied onto it once, when Formulas opens
    (said in a toast), each a file in /formulas/ as written. Add to playlist
    names the Formula: a bundled one `@formula:<stem>`, once; one of your
    own from the editor is saved first and named by its file
    (`/formulas/<stem>.psi`); the playlist editor shows both by name, as
    Formulas; the (mock) player plays them and says their names."""
    gif = card_gifs()[0]
    put_text(base + "/api/sets/psitest", gif + "\n")
    page = new_page(browser, "laptop", errors)
    page.goto(base + "/#now")
    page.evaluate("""localStorage.setItem('odmd.psi.mine', JSON.stringify([{ id: 7, name: 'Kept here', src: 'name "Kept here"\\ngray(x)' }]))""")
    page.goto(base + "/#formulas")
    open_formulas(page)
    page.wait_for_selector("#psi-mine .psi-tile[data-name='Kept here']", timeout=10000)
    t0 = last_toast(page, "ok")
    kept = overlay / "formulas" / "kept-here.psi"
    check(t0 == "1 Formula saved in this browser is on the panel now" and kept.exists() and kept.read_text() == 'name "Kept here"\ngray(x)',
          f"formulas on the panel: one saved in this browser is copied onto it once ('{t0}', {kept.exists()})")
    page.reload()
    open_formulas(page)
    page.wait_for_selector("#psi-mine .psi-tile[data-name='Kept here']", timeout=10000)
    page.wait_for_timeout(500)
    again = page.locator("#toasts .toast:has-text('on the panel now')").count()
    check(again == 0 and sorted(p.name for p in (overlay / "formulas").glob("kept-here*")) == ["kept-here.psb", "kept-here.psi"],
          f"formulas on the panel: not copied again ({again} toasts)")
    # A compiled file left from before an update (another version in its
    # header, the player and the app agreeing on the new one) is compiled again.
    kept_psb = overlay / "formulas" / "kept-here.psb"
    now_psb = kept_psb.read_bytes()
    kept_psb.write_bytes(now_psb[:3] + bytes([now_psb[3] - 1]) + now_psb[4:])
    page.reload()
    open_formulas(page)
    page.wait_for_selector("#psi-mine .psi-tile[data-name='Kept here']", timeout=10000)
    for _ in range(40):
        if kept_psb.read_bytes() == now_psb:
            break
        page.wait_for_timeout(250)
    check(kept_psb.read_bytes() == now_psb,
          f"formulas on the panel: a compiled file of an older version is compiled again (header byte {kept_psb.read_bytes()[3]}, want {now_psb[3]})")
    # A bundled one's details: Add to playlist names it.
    page.click("#psi-ex .psi-tile[data-name='Starfield']")
    page.wait_for_function("document.querySelector('#psi-side.open #psi-d-title')?.textContent == 'Starfield'", timeout=20000)
    clear_toasts(page)
    page.click("#psi-pl")
    page.wait_for_selector("#psi-pl-menu", timeout=5000)
    lists = page.eval_on_selector_all("#psi-pl-menu [data-path]", "els => els.map(e => e.dataset.path)")
    page.click("#psi-pl-menu [data-path='/sets/psitest.txt']")
    t = last_toast(page, "ok")
    lines = (overlay / "sets" / "psitest.txt").read_text().split("\n")
    want = "@formula:starfield"
    check(lists[0] == "/filelist.txt" and "/sets/psitest.txt" in lists and t == "1 added to “psitest”" and lines == [gif, want, ""],
          f"formulas playlist: Starfield added to psitest by name ('{t}'; {lines[1:]})")
    clear_toasts(page)
    page.click("#psi-pl")
    page.click("#psi-pl-menu [data-path='/sets/psitest.txt']")
    page.wait_for_selector("#toasts .toast", timeout=10000)
    t = page.eval_on_selector_all("#toasts .toast", "els => els.at(-1).textContent")
    check(t == "already in “psitest”" and (overlay / "sets" / "psitest.txt").read_text().count("@formula") == 1,
          f"formulas playlist: added again, it is there once ('{t}')")
    # The editor: one of your own is saved on the panel first, then named
    # by its file (its `-- a:` comment, retired, is a comment: `a` an
    # undeclared slider).
    formula_editor(page, "-- a: level\nloop 4\ngray(a * x + saw(t / 4) * 0.1)")
    page.fill("#psi-name", "Smoke & ramp")
    src = page.input_value("#psi-src")
    page.click("#psi-ed-pl")
    page.click("#psi-pl-menu [data-path='/sets/psitest.txt']")
    last_toast(page, "ok")
    mine = (overlay / "sets" / "psitest.txt").read_text().split("\n")[2]
    f = overlay / "formulas" / "smoke-ramp.psi"
    tick = page.inner_text("#psi-ed .tick")
    check(src.startswith('name "Smoke & ramp"\n-- a: level\n') and mine == "/formulas/smoke-ramp.psi"
          and f.exists() and f.read_text() == src and tick == "Saved ✓",
          f"formulas playlist: from the editor, saved on the panel and named by its file ({mine}; '{tick}')")
    # Saved again with a change: the same file.
    page.fill("#psi-src", src + "\n-- changed")
    page.wait_for_timeout(300)
    page.click("#psi-save")
    page.wait_for_function("document.querySelector('#psi-ed .tick')?.dataset.n >= 2", timeout=10000)
    check(f.read_text().endswith("-- changed") and sorted(p.name for p in (overlay / "formulas").glob("smoke-ramp*")) == ["smoke-ramp.psb", "smoke-ramp.psi"],
          "formulas playlist: saved again, the same file")
    # The playlist editor: by name, as Formulas.
    goto_view(page, "sets")
    page.click("#sets-list [data-path='/sets/psitest.txt'] button.pl")
    page.wait_for_selector("#set-editor", state="visible", timeout=10000)
    page.wait_for_function("document.querySelectorAll('#set-entries .vrow:not([hidden])').length >= 3", timeout=10000)
    page.wait_for_function("[...document.querySelectorAll('#set-entries .vrow .t')].some(t => t.textContent == 'Smoke & ramp')", timeout=10000)
    rows = page.eval_on_selector_all("#set-entries .vrow:not([hidden])",
                                     "els => els.map(r => [r.querySelector('.t').textContent, r.querySelector('.tag')?.textContent || ''])")
    check(entries(page) == lines[:2] + [mine] and rows[1:] == [["Starfield", "Formula"], ["Smoke & ramp", "Formula"]],
          f"formulas playlist: the playlist editor shows them by name ({rows})")
    shot(page, "formulas-playlist", "laptop")
    page.context.close()
    # The player plays them, by their names; a name not bundled is a 404.
    put_json(base + "/api/config", {"set": "psitest"})
    cur = wait_playing(base, want, 20)
    st = get_json(base + "/api/status")["now_playing"]
    check(cur == want and st["title"] == "Starfield", f"formulas playlist: the player plays it as “{st['title']}”")
    cur = wait_playing(base, mine, 30)
    st = get_json(base + "/api/status")["now_playing"]
    check(cur == mine and st["title"] == "Smoke & ramp", f"formulas playlist: and the file, as “{st['title']}”")
    put_json(base + "/api/config", {"set": ""})
    for path, code in (("@formula:no-such", 404), ("/formulas/gone.psi", 404)):
        try:
            post_json(base + "/api/play", {"path": path})
            got = 200
        except urllib.error.HTTPError as e:
            got = e.code
        check(got == code, f"formulas playlist: {path} is a {got}")
    for name in ("sets/psitest.txt", "formulas/smoke-ramp.psi", "formulas/kept-here.psi"):
        urllib.request.urlopen(urllib.request.Request(base + "/api/files/" + name, method="DELETE"), timeout=30)


def formula_keep_checks(browser, base: str, errors: list[str]) -> None:
    """Keep on the panel (config `formula_keep`): on the Formulas page and in
    Settings › Playback; a Formula shown on the panel stays past the mock's
    dwell (`now_playing.kept_s`), through a live edit; Now says so and Back
    to the playlist ends it; a GIF sent to the panel is never kept; one
    minute shows the time left."""
    page = new_page(browser, "laptop", errors)
    page.goto(base + "/#formulas")
    open_formulas(page)
    page.click("#psi-ex .psi-tile >> nth=0")
    opts = page.eval_on_selector_all("#psi-keep option", "els => els.map(e => [e.value, e.textContent])")
    check(opts == [["turn", "One turn"], ["1m", "1 minute"], ["5m", "5 minutes"], ["hold", "Until I change it"]]
          and page.input_value("#psi-keep") == "turn" and page.inner_text("#psi-keep-row").startswith("Keep on the panel"),
          f"keep: Keep on the panel in the Formula's details, one turn by default ({opts})")
    page.select_option("#psi-keep", "hold")
    deadline = time.time() + 5
    while time.time() < deadline and get_json(base + "/api/config")["formula_keep"] != "hold":
        time.sleep(0.2)
    check(get_json(base + "/api/config")["formula_keep"] == "hold", "keep: choosing Until I change it saves formula_keep=hold")
    clear_toasts(page)
    page.click("#psi-show")
    last_toast(page, "ok")
    st = get_json(base + "/api/status")["now_playing"]
    ordinal, path = st["ordinal"], st["path"]
    check(path.startswith("@formula") and st.get("kept_s") == 0, f"keep: Show on panel keeps it until changed (kept_s {st.get('kept_s')})")
    # Past the mock's 4 s dwell, and through a live edit.
    time.sleep(5)
    live = path + "&a=0.25"
    post_json(base + "/api/play", {"path": live, "live": True})
    time.sleep(1)
    st = get_json(base + "/api/status")["now_playing"]
    check(st["ordinal"] == ordinal and st["path"] == live and st.get("kept_s") == 0,
          f"keep: still on the panel past the dwell, kept through a live edit (ordinal {st['ordinal']} vs {ordinal}, kept_s {st.get('kept_s')})")
    goto_view(page, "now")
    page.wait_for_selector("#now-kept:not([hidden])", timeout=5000)
    t = page.inner_text("#now-kept-text")
    check(t == "Kept on the panel until you change it" and page.inner_text("#now-unkeep") == "Back to the playlist",
          f"keep: Now says it is kept, with Back to the playlist ('{t}')")
    shot(page, "now-kept", "laptop")
    page.click("#now-unkeep")
    page.wait_for_selector("#now-kept", state="hidden", timeout=5000)
    st = get_json(base + "/api/status")["now_playing"]
    check(st["ordinal"] != ordinal and not st["path"].startswith("@formula") and "kept_s" not in st,
          f"keep: Back to the playlist moves on and the keep ends ({st['path'][:40]})")
    # A GIF sent to the panel is never kept.
    gif = card_gifs()[0]
    post_json(base + "/api/play", {"path": gif})
    st = get_json(base + "/api/status")["now_playing"]
    check(st["path"] == gif and "kept_s" not in st, "keep: a GIF sent to the panel plays one turn as before")
    # One minute: the time left on Now.
    put_json(base + "/api/config", {"formula_keep": "1m"})
    post_json(base + "/api/play", {"path": "@formula:starfield"})
    page.wait_for_selector("#now-kept:not([hidden])", timeout=5000)
    page.wait_for_function("/^Kept on the panel: [01]:\\d\\d left$/.test(document.querySelector('#now-kept-text').textContent)", timeout=5000)
    t = page.inner_text("#now-kept-text")
    check(get_json(base + "/api/status")["now_playing"].get("kept_s") == 60 and t in ("Kept on the panel: 1:00 left", "Kept on the panel: 0:59 left", "Kept on the panel: 0:58 left"),
          f"keep: a minute shows the time left ('{t}')")
    # Settings › Playback has the same setting (a fresh load: the minute was set by the API).
    page.goto("about:blank")
    page.goto(base + "/#settings/play")
    page.wait_for_selector("#st-keep", timeout=10000)
    check(page.input_value("#st-keep") == "1m", f"keep: Settings › Playback shows it ({page.input_value('#st-keep')})")
    page.select_option("#st-keep", "turn")
    deadline = time.time() + 5
    while time.time() < deadline and get_json(base + "/api/config")["formula_keep"] != "turn":
        time.sleep(0.2)
    check(get_json(base + "/api/config")["formula_keep"] == "turn", "keep: Settings › Playback saves it (back to one turn)")
    try:
        put_json(base + "/api/config", {"formula_keep": "forever"})
        refused = False
    except Exception as e:  # noqa: BLE001 (the mock's 400)
        refused = "400" in str(e)
    check(refused, "keep: formula_keep refuses a value it doesn't know")
    post_json(base + "/api/skip", {})
    page.context.close()


IMPORT_SEND = """([u, b64, fields, size]) => {
    const bytes = b64 ? Uint8Array.from(atob(b64), (c) => c.charCodeAt(0)) : new Uint8Array(size);
    if (!b64) bytes.set([71, 73, 70, 56]);
    addEventListener('message', function on(e) {
        if (e.data?.type !== 'opendmd-import-ready') return;
        removeEventListener('message', on);
        const buf = bytes.buffer;
        e.source.postMessage({ type: 'opendmd-import', gif: buf, ...fields }, '*', [buf]);
        window.__sent = buf.byteLength;  // 0 once transferred
    });
    window.open(u, '_blank');
}"""


def import_open(opener: Page, base: str, errors: list[str], gif: bytes | None, fields: dict, size: int = 0) -> Page:
    """The panel's app opened at `#import` by `opener` (as the site's Create
    Art opens it with Send to my panel), its ready answered with a GIF
    (`gif`, or `size` zero bytes after a GIF header) transferred by postMessage."""
    import base64
    with opener.context.expect_page() as info:
        opener.evaluate(IMPORT_SEND, [base + "/#import", base64.b64encode(gif).decode() if gif else "", fields, size])
    app = info.value
    app.on("pageerror", lambda e: errors.append(f"[import] pageerror: {e}"))
    app.on("dialog", lambda d: d.dismiss())  # (the playlist offer: not this time)
    return app


def import_checks(browser, base: str, overlay: Path, errors: list[str]) -> None:
    """Send to my panel from the site's Create Art: the app opened at
    `#import` says it is ready to its opener, takes the GIF it sends (a
    transferred ArrayBuffer, from opendmd.dev or the app's own origin) and
    opens the Library's Add GIFs with the file, its title, platform, author
    and logo filled in; Add puts it and its prepared copy on the card, there
    after a reload. Refused, and said: no opener, too big, not a GIF; an
    opener on another origin is ignored."""
    from PIL import Image, ImageDraw
    frames = []
    for i in range(3):
        im = Image.new("RGB", (128, 32), (10, 20, 80))
        ImageDraw.Draw(im).rectangle([10 + 30 * i, 8, 40 + 30 * i, 24], fill=(240, 200, 40))
        frames.append(im)
    import io
    b = io.BytesIO()
    frames[0].save(b, "GIF", save_all=True, append_images=frames[1:], duration=120, loop=0)
    gif = b.getvalue()
    opener = new_page(browser, "laptop", errors)
    opener.goto(base + "/#now")
    opener.wait_for_selector("#view-now.on", timeout=20000)
    fields = {"name": "SNES_Import_Test_LOGO_01_tester1.gif", "title": "Import Test", "platform": "SNES", "author": "tester1", "logo": True}
    app = import_open(opener, base, errors, gif, fields)
    app.wait_for_selector("#dl .dl-item", timeout=30000)
    got = app.evaluate("""() => { const i = document.querySelector('#dl .dl-item'); return {title: i.querySelector('.dl-title').value,
        platform: i.querySelector('.dl-plat').value, author: i.querySelector('.dl-author').value, img: !!i.querySelector('img'), file: i.textContent}; }""")
    check(app.url.endswith("#library") and got["title"] == "Import Test" and got["platform"] == "SNES" and got["author"] == "tester1"
          and got["img"] and opener.evaluate("window.__sent") == 0,
          f"import: #import -> Add GIFs filled in from what the site sent, the GIF transferred ({got['title']}, {got['platform']}, {got['author']})")
    app.select_option("#dl .dl-cat", "Other")
    app.wait_for_function("document.querySelector('#dl .dl-item').dataset.name && !document.querySelector('#dl-ok').disabled", timeout=10000)
    name = app.get_attribute("#dl .dl-item", "data-name")
    path = f"/gifs/Other/{name}"
    parsed = app.evaluate(f"import('./js/state.js').then(m => m.app.wasm.parsePath({json.dumps(path)}))")
    check((parsed["platform"], parsed["title"], "LOGO" in parsed["flags"], "tester1" in parsed["flags"]) == ("SNES", "Import Test", True, True),
          f"import: named as the site named it, the logo and author kept ({name})")
    app.click("#dl-ok")
    f, copy = overlay / path.lstrip("/"), overlay / cache_path(path).lstrip("/")
    deadline = time.time() + 60
    while time.time() < deadline and not (f.exists() and copy.exists()):
        time.sleep(0.2)
    check(f.exists() and f.read_bytes() == gif and copy.exists(), f"import: Add -> {path} on the card, byte for byte, and its prepared copy")
    app.reload()
    app.wait_for_selector("#view-library.on", timeout=20000)
    check(app.locator("#dl").count() == 0, "import: a reload doesn't ask again")
    lib_row(app, path)
    check(True, "import: after a reload the Library lists it")
    no_leaks(app, "import")
    app.close()
    # Too big, and not a GIF: said, nothing added.
    app = import_open(opener, base, errors, None, {"name": "Big.gif", "title": "Big"}, size=4 * 1024 * 1024 + 1)
    t = toast_or_none(app, "bad", 20000)
    check("over 4 MB" in t and app.locator("#dl").count() == 0, f"import: over 4 MB refused, said ('{t}')")
    app.close()
    app = import_open(opener, base, errors, b"PNG not a gif at all", {"name": "x.gif"})
    t = toast_or_none(app, "bad", 20000)
    check("isn’t a GIF" in t and app.locator("#dl").count() == 0, f"import: not a GIF refused, said ('{t}')")
    app.close()
    opener.context.close()
    # An opener on another site: ignored.
    other = new_page(browser, "laptop", errors)
    other.context.route("http://elsewhere.test/**", lambda r: r.fulfill(status=200, headers={"content-type": "text/html"}, body="<!doctype html><p>elsewhere"))
    other.goto("http://elsewhere.test/")
    app = import_open(other, base, errors, gif, fields)
    app.wait_for_selector("#view-library.on", timeout=20000)
    app.wait_for_timeout(3000)
    check(app.locator("#dl").count() == 0 and other.evaluate("window.__sent") == 0, "import: a GIF sent from another site is ignored")
    other.context.close()
    # No opener: said.
    page = new_page(browser, "laptop", errors)
    page.goto(base + "/#import")
    t = toast_or_none(page, "bad", 20000)
    check("open it again from Create Art" in t and page.url.endswith("#library"), f"import: no opener -> '{t}'")
    page.context.close()
    # Leave the card as it was.
    urllib.request.urlopen(urllib.request.Request(base + "/api/files" + path, method="DELETE"), timeout=30).read()


def formulas_remix_checks(browser, base: str, overlay: Path, errors: list[str]) -> None:
    """Open on my panel: the site hands a Formula to the panel's app by name
    (`#gallery=<slug>`: a bundled one is built in, another fetched from
    opendmd.dev/formulas/src/, here a stand-in) or, edited, by postMessage
    from the page that opened it (`#remix`, from opendmd.dev or the app's
    own origin only), never in the link. It is saved on the panel as it
    arrives (under Your formulas, kept over a reload; the same text not
    twice; a bundled one needs nothing; one that doesn't work isn't saved,
    and comes back as not saved yet) and opens in the editor; a name it
    doesn't have, no opener, or an opener elsewhere: said plainly."""
    raw = (REPO / "opendmd-web/formulas/searchlight.psi").read_text()
    src = raw.split("\n", 2)[2]
    f = overlay / "formulas" / "searchlight.psi"
    if f.exists():
        f.unlink()
    page = new_page(browser, "laptop", errors)
    asked: list[str] = []

    def site(route):
        asked.append(route.request.url)
        slug = route.request.url.rsplit("/", 1)[1]
        if slug == "searchlight.psi":
            route.fulfill(status=200, body=raw, headers={"content-type": "text/plain; charset=utf-8", "access-control-allow-origin": "*"})
        else:
            route.fulfill(status=404, body="no", headers={"access-control-allow-origin": "*"})
    page.context.route("https://opendmd.dev/formulas/src/*", site)
    # A gallery Formula the panel doesn't come with, a slider moved on the site.
    page.goto(base + "/#gallery=searchlight&a=0.6")
    page.wait_for_selector("#psi-ed #psi-src", timeout=20000)
    page.wait_for_timeout(500)
    got = (page.input_value("#psi-src"), page.input_value("#psi-name"), page.input_value("#psi-ed-sl-a"), page.evaluate("location.hash"))
    check(got == (src, "Searchlight", "0.6", "#formulas/edit") and asked == ["https://opendmd.dev/formulas/src/searchlight.psi"],
          f"remix: a gallery Formula is fetched by name and opens in the editor ({got[1:]}, {asked})")
    # Saved on the panel as it arrives (no Save pressed), and said.
    t = toast_or_none(page, "ok")
    page.wait_for_timeout(300)
    check(f.exists() and f.read_text() == src and "Your formulas" in t,
          f"remix: a gallery Formula is saved on the panel as it arrives (/formulas/searchlight.psi, '{t}')")
    shot(page, "formulas-remix", "laptop")
    # Still there after a reload, under Your formulas, with the slider as sent.
    page.goto(base + "/#formulas")
    page.reload()
    page.wait_for_selector("#psi-ex .psi-tile", timeout=20000)
    check(page.eval_on_selector_all("#psi-mine .psi-tile", "els => els.map(e => e.dataset.name)").count("Searchlight") == 1,
          "remix: after a reload the gallery Formula is under Your formulas")
    # A change saved later goes into the same file, not a second one.
    if page.locator("#psi-mine .psi-tile[data-name='Searchlight']").count():
        page.click("#psi-mine .psi-tile[data-name='Searchlight']")
        page.click("#psi-edit")
        page.wait_for_selector("#psi-ed #psi-src", timeout=10000)
        page.fill("#psi-src", src.replace("0.4", "0.5", 1))
        page.wait_for_timeout(300)
        page.click("#psi-save")
        page.wait_for_selector("#psi-ed .tick.ok", timeout=5000)
    check(f.exists() and f.read_text() == src.replace("0.4", "0.5", 1) and sorted(p.name for p in (overlay / "formulas").glob("searchlight*.psi")) == ["searchlight.psi"],
          "remix: Save afterwards changes the same file (/formulas/searchlight.psi, no second copy)")
    # Edited on the site (Open on my panel, a changed one): sent by postMessage,
    # saved as it arrives, kept over a reload and a trip to another view.
    sent_src = 'name "Sent rings"\nloop 4\ngray(saw(x - t/4))'
    sf = overlay / "formulas" / "sent-rings.psi"
    opener = new_page(browser, "laptop", errors)
    opener.goto(base + "/version.txt")
    app = remix_open(opener, base, errors, sent_src, "Sent rings", a=0.25)
    t = toast_or_none(app, "ok")
    app.wait_for_timeout(300)
    check(sf.exists() and sf.read_text() == sent_src and "Your formulas" in t,
          f"remix: a Formula sent from the site is saved on the panel as it arrives (/formulas/sent-rings.psi, '{t}')")
    app.goto(base + "/#library")
    app.wait_for_timeout(500)
    app.goto(base + "/#formulas")
    app.reload()
    app.wait_for_selector("#psi-ex .psi-tile", timeout=20000)
    mine = app.eval_on_selector_all("#psi-mine .psi-tile", "els => els.map(e => e.dataset.name)")
    check(mine.count("Searchlight") == 1 and mine.count("Sent rings") == 1, f"remix: after another view and a reload it is under Your formulas ({mine})")
    opener.context.close()
    # The same Formula sent again: opened, not saved twice.
    opener = new_page(browser, "laptop", errors)
    opener.goto(base + "/version.txt")
    app = remix_open(opener, base, errors, sent_src, "Sent rings")
    app.wait_for_timeout(500)
    check(sorted(p.name for p in (overlay / "formulas").glob("sent-rings*.psi")) == ["sent-rings.psi"] and app.input_value("#psi-name") == "Sent rings",
          "remix: the same Formula sent again opens the saved one, no second copy")
    opener.context.close()
    # One that doesn't work: not saved, said, and offered back as not saved yet.
    opener = new_page(browser, "laptop", errors)
    opener.goto(base + "/version.txt")
    app = remix_open(opener, base, errors, 'name "Broken rings"\ngray(', "Broken rings")
    t = toast_or_none(app, "bad")
    check(not (overlay / "formulas" / "broken-rings.psi").exists() and "isn’t saved" in t,
          f"remix: a Formula that doesn't work isn't saved, and says so ('{t}')")
    app.goto(base + "/#formulas")
    app.reload()
    app.wait_for_selector("#psi-ex .psi-tile", timeout=20000)
    cont = app.inner_text("#psi-cont") if app.locator("#psi-cont").count() else ""
    check("Broken rings" in cont, f"remix: after a reload it is offered back, not saved yet ('{cont[:60]}')")
    if cont:
        app.click("#psi-cont-drop")
    opener.context.close()
    # A bundled one: here already, nothing fetched, nothing saved.
    asked.clear()
    before = sorted(p.name for p in (overlay / "formulas").glob("*"))
    page.goto("about:blank")
    page.goto(base + "/#gallery=starfield")
    page.wait_for_selector("#psi-ed #psi-src", timeout=20000)
    page.wait_for_timeout(500)
    star = next(e for e in bundled() if e["name"] == "Starfield")
    after = sorted(p.name for p in (overlay / "formulas").glob("*"))
    check(page.input_value("#psi-src") == star["src"] and not asked and after == before,
          f"remix: a bundled one opens from the panel's own copy, nothing saved ({asked}, {after})")
    # A name the gallery doesn't have (its 404 expected).
    expected.append("/formulas/src/no-such-formula")
    page.goto("about:blank")
    page.goto(base + "/#gallery=no-such-formula")
    t = last_toast(page, "bad")
    check(t == "Couldn’t open that Formula: the gallery has no Formula by that name" and page.evaluate("location.hash") == "#formulas",
          f"remix: a name the gallery doesn't have says so ('{t}')")
    # `#remix` with no page that opened it.
    page.goto("about:blank")
    page.goto(base + "/#remix")
    t = last_toast(page, "bad")
    check(t == "Couldn’t open that Formula: open it again from opendmd.dev", f"remix: no page sending it says so ('{t}')")
    # Sent from another site: ignored, and said after the wait.
    other = new_page(browser, "laptop", errors)
    other.context.route("http://elsewhere.test/**", lambda r: r.fulfill(status=200, headers={"content-type": "text/html"}, body=(
        "<!doctype html><script>addEventListener('message', (e) => { if (e.data?.type === 'opendmd-remix-ready') "
        "e.source.postMessage({type: 'opendmd-remix', name: 'Elsewhere', src: 'gray(1)'}, '*'); });</script>")))
    other.goto("http://elsewhere.test/")
    with other.context.expect_page() as info:
        other.evaluate("u => { window.open(u, '_blank'); }", base + "/#remix")
    app = info.value
    app.wait_for_selector("#toasts .toast.bad", timeout=20000)
    t = app.eval_on_selector_all("#toasts .toast.bad", "els => els.at(-1).textContent")
    check("didn’t send it" in t and app.input_value("#psi-name") != "Elsewhere" if app.locator("#psi-name").count() else "didn’t send it" in t,
          f"remix: a Formula sent from another site is ignored ('{t}')")
    other.context.close()
    page.context.close()
    for p in [*(overlay / "formulas").glob("searchlight*.psi"), *(overlay / "formulas").glob("sent-rings*.psi")]:
        urllib.request.urlopen(urllib.request.Request(base + f"/api/files/formulas/{p.name}", method="DELETE"), timeout=30)


def formulas_store_checks(browser, base: str, overlay: Path, errors: list[str]) -> None:
    """Your Formulas on the panel when the panel can't list them: said, not
    shown as none, and nothing saved over one already there (a save waits
    for a listing that works); once it lists again, the one there is whole."""
    d = overlay / "formulas"
    d.mkdir(parents=True, exist_ok=True)
    keep = d / "kept-rings.psi"
    kept = 'name "Kept rings"\nloop 4\ngray(saw(x + t/4))'
    keep.write_text(kept)
    expected.append("/api/files?dir=%2Fformulas")
    try:
        opener = new_page(browser, "laptop", errors)
        fail = lambda r: r.fulfill(status=500, body="card error", headers={"content-type": "text/plain"})
        opener.context.route("**/api/files?dir=%2Fformulas", fail)
        opener.goto(base + "/#formulas")
        t = toast_or_none(opener, "bad")
        check("Couldn’t read your Formulas" in t and opener.locator("#psi-mine .psi-tile").count() == 0,
              f"store: a listing that fails is said, not shown as no Formulas ('{t}')")
        opener.goto(base + "/version.txt")
        app = remix_open(opener, base, errors, 'name "Kept rings"\nloop 4\ngray(1)', "Kept rings")
        t = toast_or_none(app, "bad")
        check(keep.read_text() == kept and sorted(p.name for p in d.glob("kept-rings*")) == ["kept-rings.psi"] and "isn’t saved" in t,
              f"store: with no listing nothing is saved over a Formula already there ('{t}')")
        opener.context.unroute("**/api/files?dir=%2Fformulas", fail)
        app.goto(base + "/#formulas")
        app.reload()
        app.wait_for_selector("#psi-ex .psi-tile", timeout=20000)
        mine = app.eval_on_selector_all("#psi-mine .psi-tile", "els => els.map(e => e.dataset.name)")
        check(mine.count("Kept rings") == 1 and keep.read_text() == kept, f"store: listed again, the one there is whole ({mine})")
        if app.locator("#psi-cont-drop").count():
            app.click("#psi-cont-drop")
        opener.context.close()
    finally:
        expected.remove("/api/files?dir=%2Fformulas")
        for p in d.glob("kept-rings*"):
            p.unlink()


def clear_toasts(page: Page) -> None:
    page.evaluate("document.querySelectorAll('#toasts .toast').forEach(t => t.remove())")


def formulas_panel_checks(browser, base: str, errors: list[str]) -> None:
    """Show on panel: a formula plays on the (mock) panel as `@formula?psb=…`;
    slider changes follow it there, ~200 ms apart and one at a time, as live
    edits (the same ordinal); once the playlist moves on a change stops
    following (409, no toast); an older firmware (415), a formula it refuses
    (400) and no answer each say so."""
    page = new_page(browser, "laptop", errors)
    expected.append("/api/play")  # the 409, 415, 400 and the refused request below
    plays: list[tuple[float, dict]] = []
    page.on("request", lambda r: plays.append((time.time(), json.loads(r.post_data))) if r.url.endswith("/api/play") else None)
    page.goto(base + "/#formulas")
    open_formulas(page)
    page.click("#psi-ex .psi-tile >> nth=0")
    name = page.inner_text("#psi-d-title")
    check(page.inner_text("#psi-show") == "Show on panel", "formulas panel: Show on panel in Details")
    # The path is the device's own (`fx_psi::path_of`): the compiled Formula.
    src = 'name "Ü"\nv = x % 2 + 1 // c\ngray(v)'
    enc = page.evaluate("(s) => import('./js/psishow.js').then(m => m.fpath({src: s, a: 0.5}))", src)
    psb = enc[len("@formula?psb="):-len("&a=0.5")] if enc.startswith("@formula?psb=") and enc.endswith("&a=0.5") else ""
    raw = base64.urlsafe_b64decode(psb + "=" * (-len(psb) % 4)) if psb else b""
    check(raw[:4] == b"PSB\x02" and raw[6:6 + raw[5]].decode() == "Ü", f"formulas panel: the formula's path as the device reads it ({enc[:60]})")
    clear_toasts(page)
    page.click("#psi-show")
    t = last_toast(page, "ok")
    st = get_json(base + "/api/status")["now_playing"]
    check(t == f"On the panel: {name}" and st["path"].startswith("@formula?psb=") and st["title"] == name
          and page.inner_text("#psi-show") == "On the panel ✓" and plays[-1][1].get("live") is False,
          f"formulas panel: {name} on the panel ('{t}', {st['path'][:40]}…, {st['title']})")
    shot(page, "formulas-panel-on", "laptop")
    # In the editor: a slider dragged for ~0.6 s follows it, throttled.
    formula_editor(page, "-- a: level\nloop 4\ngray(a * x + saw(t / 4) * 0.1)")
    page.click("#psi-ed-show")
    page.wait_for_function("document.querySelector('#psi-ed-show').textContent == 'On the panel ✓'", timeout=5000)
    ordinal = get_json(base + "/api/status")["now_playing"]["ordinal"]
    n0, t0 = len(plays), time.time()
    page.evaluate("""async () => { const r = document.querySelector('#psi-ed-sl-a');
        for (let k = 1; k <= 60; k++) { r.value = k / 60; r.dispatchEvent(new Event('input')); await new Promise(f => setTimeout(f, 10)); } }""")
    page.wait_for_timeout(800)
    sent = plays[n0:]
    gaps = [b[0] - a[0] for a, b in zip(sent, sent[1:])]
    st = get_json(base + "/api/status")["now_playing"]
    check(2 <= len(sent) <= 6 and all(p.get("live") is True for _, p in sent) and min(gaps, default=1) > 0.15
          and sent[-1][1]["path"].endswith("&a=1") and st["ordinal"] == ordinal and st["path"] == sent[-1][1]["path"],
          f"formulas panel: 60 slider moves in {time.time() - t0:.1f} s -> {len(sent)} live edits, gaps {[round(g, 2) for g in gaps]}, the last a=1 on the panel (ordinal {st['ordinal']})")
    # The playlist moves on (the mock's 4 s dwell): a change stops following, quietly.
    deadline = time.time() + 10
    while time.time() < deadline and get_json(base + "/api/status")["now_playing"]["path"].startswith("@formula"):
        time.sleep(0.3)
    clear_toasts(page)
    n0 = len(plays)
    page.eval_on_selector("#psi-ed-sl-a", "r => { r.value = 0.5; r.dispatchEvent(new Event('input')); }")
    page.wait_for_function("document.querySelector('#psi-ed-show').textContent == 'Show on panel'", timeout=5000)
    bad = page.eval_on_selector_all("#toasts .toast.bad", "els => els.length")
    check(len(plays) == n0 + 1 and plays[-1][1].get("live") is True and bad == 0
          and not get_json(base + "/api/status")["now_playing"]["path"].startswith("@formula"),
          "formulas panel: after the playlist moved on, a change does not bring it back (409, no error shown)")
    # What goes wrong, said plainly.
    for status, body, want in ((415, "@formula: not a playable file", "Update OpenDMD to show formulas on the panel"),
                               (400, "unknown name `q`", "The panel can’t show"),
                               (0, "", "Couldn’t reach the panel")):
        def answer(route, _req, status=status, body=body):
            if not status:
                route.abort()
            else:
                route.fulfill(status=status, body=json.dumps({"error": body}), headers={"content-type": "application/json"})
        page.route("**/api/play", answer)
        clear_toasts(page)
        page.click("#psi-ed-show")
        t = last_toast(page, "bad")
        page.unroute("**/api/play")
        check(t.startswith(want) and page.inner_text("#psi-ed-show") == "Show on panel", f"formulas panel: a {status or 'failed request'} says '{t}'")
    shot(page, "formulas-panel", "laptop")
    page.context.close()
    expected.remove("/api/play")


def formulas_over_checks(browser, base: str, errors: list[str]) -> None:
    """Formulas over a GIF: an example that draws over one plays over the
    sample picture, then over a GIF picked from the playlist (decoded by
    pkg-art) and changes with it; Show over the panel's GIF sends
    `"over":true` and takes it off again; Set as the panel's look (it
    declares `look`) sets the config, Settings › Look lists it and None
    clears it; Save as GIF over the GIF."""
    page = new_page(browser, "laptop", errors)
    plays: list[dict] = []
    page.on("request", lambda r: plays.append(json.loads(r.post_data)) if r.url.endswith("/api/play") else None)
    page.goto(base + "/#formulas")
    open_formulas(page)
    page.click("#psi-ex .psi-tile[data-name='Old photo']")
    wait_lit(page, "#psi-panel", "formulas over: Old photo in Details")
    check(page.is_visible("#psi-under") and page.inner_text("#psi-under-name") == "a sample picture"
          and page.inner_text("#psi-show") == "Show over the panel’s GIF" and page.inner_text("#psi-look") == "Set as the panel’s look"
          and page.is_visible("#psi-ex .psi-tile[data-name='Old photo'] .psi-badge"),
          f"formulas over: drawn over the sample, Show over the panel’s GIF, Set as the panel’s look (it declares `look`) ({page.inner_text('#psi-show')!r})")
    s0 = canvas_sum(page, "#psi-panel")
    page.click("#psi-under-pick")
    page.wait_for_selector("#psi-glist .mi", timeout=10000)
    first = page.inner_text("#psi-glist .mi >> nth=0")
    page.click("#psi-glist .mi >> nth=0")
    page.wait_for_function(f"document.querySelector('#psi-under-name').textContent == {json.dumps(first)}", timeout=30000)
    page.wait_for_timeout(500)
    s1 = canvas_sum(page, "#psi-panel")
    page.wait_for_timeout(700)
    s2 = canvas_sum(page, "#psi-panel")
    check(s1 != s0, f"formulas over: a GIF from the playlist ('{first}') is under it now ({s0} -> {s1}, then {s2})")
    # On the (mock) panel: over its GIF, then off again.
    clear_toasts(page)
    page.click("#psi-show")
    t = last_toast(page, "ok")
    fl = get_json(base + "/api/status")["formula_look"]
    check(t == "Over the panel’s GIF: Old photo" and plays[-1].get("over") is True and plays[-1]["path"].startswith("@formula?psb=")
          and (fl or {}).get("source") == "over_gif" and page.inner_text("#psi-show") == "Over the panel’s GIF ✓",
          f"formulas over: shown over the panel's GIF ('{t}', {fl})")
    clear_toasts(page)
    page.click("#psi-show")
    t = last_toast(page, "ok")
    check(t == "Taken off the panel" and plays[-1] == {"path": "", "live": False, "over": True}
          and get_json(base + "/api/status")["formula_look"] is None and page.inner_text("#psi-show") == "Show over the panel’s GIF",
          f"formulas over: pressed again, it comes off ('{t}', {plays[-1]})")
    # Kept as the look (its own `look` line: over, 100 %); Settings › Look
    # lists it; None clears it. A Formula without the line has no button.
    clear_toasts(page)
    page.click("#psi-look")
    t = last_toast(page, "ok")
    c = get_json(base + "/api/config")
    fl = get_json(base + "/api/status")["formula_look"]
    # A bundled Formula: named, as the player has it built in.
    same = "@formula:old-photo"
    page.click("#psi-ex .psi-tile[data-name='Matrix']")
    plain = page.is_visible("#psi-look")
    check(c["look_formula"] == same and not plain
          and (fl or {}).get("source") == "setting" and t.startswith("“Old photo” is the panel’s look now"),
          f"formulas over: Set as the panel’s look ('{t}', {fl}; Matrix has the button: {plain})")
    goto_view(page, "settings")
    page.evaluate("location.replace('#settings/look')")
    page.wait_for_function("document.querySelector('#st-look-formula').selectedOptions[0]?.textContent == 'Old photo'", timeout=10000)
    opts = page.eval_on_selector_all("#st-look-formula option", "els => els.map(e => e.textContent)")
    page.select_option("#st-look-formula", "")
    page.wait_for_selector("#st-tab-look .tick.ok", timeout=10000)
    c = get_json(base + "/api/config")
    check(c["look_formula"] == "" and "Scanlines" in opts and opts[0] == "None",
          f"formulas over: Settings › Look lists the looks ({len(opts)}) and None turns it off")
    # Save as GIF over the picked GIF: Create Art, titled with both.
    goto_view(page, "formulas")
    page.click("#psi-ex .psi-tile[data-name='Old photo']")
    page.click("#psi-gif")
    page.wait_for_selector("#view-create.on #studio .st-grid", timeout=20000)
    page.wait_for_function("(d => d.dataset.animated && d.dataset.name)(document.querySelector('#studio'))", timeout=30000)
    d = studio_data(page)
    check(d.get("name", "").startswith("FX_Old_photo_over_"), f"formulas over: Save as GIF over the GIF ({d.get('name')}, {d.get('srcFrames')} frames)")
    shot(page, "formulas-over", "laptop")
    page.context.close()


def looks_checks(browser, base: str, errors: list[str]) -> None:
    """A Formula as the panel's look (formula-apps F2): Settings › Look
    lists only Formulas that declare `look`, with no blend or amount of its
    own, and saves the Formula's path; one saved elsewhere is listed by its
    `name` line with how its `look` line goes on, and Change… opens its
    settings; a refusal puts the picker back; the old fields are gone from
    the config. The editor's Settings sheet writes name, about, controls
    and the look as lines of the text (the header follows), refuses Over
    100 % of a covering Formula in a sentence, and Set as the panel's look
    sends the path; links and paths leave out values at their defaults."""
    page = new_page(browser, "laptop", errors)
    page.goto(base + "/#settings/look")
    page.wait_for_function("[...document.querySelectorAll('#st-look-formula option')].some(o => o.textContent == 'Scanlines')", timeout=20000)
    opts = page.eval_on_selector_all("#st-look-formula option", "els => els.map(e => [e.textContent, e.disabled, e.parentElement.label || ''])")
    names = [o[0] for o in opts]
    looks = {"Scanlines", "Old TV", "Vignette", "Old photo", "Glitter"}
    check(names[0] == "None" and set(names[1:]) == looks and all(o[2] == "Bundled Formulas" and not o[1] for o in opts[1:])
          and page.locator("#st-look-blend, #st-look-amt").count() == 0,
          f"looks: Settings › Look lists the bundled Formulas that declare `look`, no blend or amount ({names})")
    page.select_option("#st-look-formula", label="Vignette")
    page.wait_for_selector("#st-tab-look .tick.ok", timeout=10000)
    c = get_json(base + "/api/config")
    want = "@formula:vignette"
    why = page.inner_text("#st-look-why")
    check(c["look_formula"] == want and why.startswith("Over at 100%") and page.is_visible("#st-look-change")
          and not {"look_formula_name", "look_blend", "look_amount"} & set(c),
          f"looks: Vignette saved by its name ('{why}'; {c['look_formula']!r})")
    shot(page, "looks-settings", "laptop")
    # A refusal by the player: the picker goes back to what it has.
    def refuse(route):
        if route.request.method == "PUT":
            route.fulfill(status=400, body=json.dumps({"error": "look_formula: refused for the test"}), headers={"content-type": "application/json"})
        else:
            route.continue_()
    page.route("**/api/config", refuse)
    expected.append("/api/config")  # the refusal's 400, on purpose
    page.select_option("#st-look-formula", label="Scanlines")
    page.wait_for_function("document.querySelector('#st-look-formula').selectedOptions[0]?.textContent == 'Vignette'", timeout=10000)
    tick = page.inner_text("#st-tab-look .tick")
    page.unroute("**/api/config", refuse)
    page.wait_for_timeout(300)
    expected.remove("/api/config")
    check("refused for the test" in tick, f"looks: a refusal puts the picker back (Vignette, '{tick}')")
    # A look saved elsewhere: listed by its own name, said how it goes on.
    gone_src = 'name "Gone"\nlook add 25\nloop 4\nhsv(x + t/4, 1, 1)'
    gone = page.evaluate("s => import('./js/psishow.js').then(m => m.fpath({ src: s }))", gone_src)
    put_json(base + "/api/config", {"look_formula": gone, "look_blend": "screen", "look_formula_name": "Stale"})
    c = get_json(base + "/api/config")
    page.reload()
    page.wait_for_function("document.querySelector('#st-look-formula')?.selectedOptions[0]?.textContent == 'Gone'", timeout=20000)
    why = page.inner_text("#st-look-why")
    check(c["look_formula"] == gone and "look_blend" not in c and why == "Set elsewhere: open it where it was made to change it"
          and page.is_hidden("#st-look-change"),
          f"looks: a look saved elsewhere is listed by its name ('{why}'; old fields ignored: {sorted(set(c) & {'look_blend', 'look_formula_name'})})")
    # The sheet over Matrix (a remix): every change a line of the text.
    open_formulas(page)
    page.click("#psi-ex .psi-tile[data-name='Matrix']")
    page.click("#psi-settings")
    if page.locator("#psi-ask").count():
        page.click("#psi-ask button:has-text('Replace it')")
    page.wait_for_selector("#psi-set #psi-set-name", timeout=10000)
    src0 = page.input_value("#psi-src")
    page.fill("#psi-set-name", "Matrix glow")
    page.fill("#psi-set-about", "Matrix, screened over the GIF")
    page.check("#psi-set-look")
    page.wait_for_timeout(300)
    mix = (page.input_value("#psi-set-lk-blend"), page.input_value("#psi-set-lk-amt"))
    page.select_option("#psi-set-lk-blend", "add")
    page.eval_on_selector("#psi-set-lk-amt", "r => { r.value = 50; r.dispatchEvent(new Event('input')); r.dispatchEvent(new Event('change')); }")
    page.click("#psi-set-add")
    page.fill("#psi-set-c0-label", "Glow")
    page.dispatch_event("#psi-set-c0-label", "change")
    page.wait_for_timeout(300)
    src = page.input_value("#psi-src")
    head = src.split("\n")[:4]
    check(src0.startswith('name "Matrix remix"\nabout "Green code') and head == ['name "Matrix glow"', 'about "Matrix, screened over the GIF"', 'control k1 0..1 = 0.5 "Glow"', "look add 50"]
          and mix == ("screen", "40") and page.input_value("#psi-name") == "Matrix glow" and page.is_visible("#psi-set-c0 .psi-sl"),
          f"looks: the sheet writes the lines ({head}; the look started at {mix})")
    shot(page, "formula-settings", "laptop")
    # A choice, then gone again.
    page.select_option("#psi-set-c0-kind", "c")
    page.wait_for_timeout(300)
    l1 = page.input_value("#psi-src").split("\n")[2]
    page.click("#psi-set-c0-del")
    page.wait_for_timeout(300)
    gone_ctl = "control" not in page.input_value("#psi-src")
    check(l1 == 'control k1 = on, off "Glow"' and gone_ctl and page.locator("#psi-set-c0").count() == 0,
          f"looks: a control made a choice ({l1!r}), then removed ({gone_ctl})")
    # Over 100 % of a Formula that covers the GIF: said, not offered.
    page.select_option("#psi-set-lk-blend", "over")
    page.eval_on_selector("#psi-set-lk-amt", "r => { r.value = 100; r.dispatchEvent(new Event('input')); r.dispatchEvent(new Event('change')); }")
    page.wait_for_timeout(300)
    why = page.inner_text("#psi-set-lk-why")
    off = page.is_disabled("#psi-set-use")
    line = [l for l in page.input_value("#psi-src").split("\n") if l.startswith("look")]
    check("covers the GIF entirely" in why and off and line == ["look"], f"looks: Over 100% of Matrix is refused in a sentence ('{why}', {line})")
    page.select_option("#psi-set-lk-blend", "screen")
    page.eval_on_selector("#psi-set-lk-amt", "r => { r.value = 40; r.dispatchEvent(new Event('input')); r.dispatchEvent(new Event('change')); }")
    page.wait_for_timeout(300)
    clear_toasts(page)
    page.click("#psi-set-use")
    t = last_toast(page, "ok")
    c5 = get_json(base + "/api/config")
    st = get_json(base + "/api/status")
    saved = get_text(base + "/formulas/matrix-glow.psi") if c5["look_formula"] == "/formulas/matrix-glow.psi" else ""
    check(c5["look_formula"] == "/formulas/matrix-glow.psi" and saved.startswith('name "Matrix glow"\nabout "Matrix, screened over the GIF"\n') and "\nlook screen 40\n" in saved
          and t.startswith("“Matrix glow” is the panel’s look now") and (st.get("formula_look") or {}).get("source") == "setting",
          f"looks: Set as the panel’s look saves it on the panel and names its file ('{t}'; {c5['look_formula']!r})")
    compiled = urllib.request.urlopen(base + "/formulas/matrix-glow.psb", timeout=30).read() if saved else b""
    check(compiled[:4] == b"PSB\x02", "looks: the saved Formula's compiled file is beside it")
    # Off: the line goes.
    page.click("#psi-set-btn")
    page.wait_for_selector("#psi-set #psi-set-look", timeout=10000)
    page.uncheck("#psi-set-look")
    page.wait_for_timeout(300)
    check(not any(l.startswith("look") for l in page.input_value("#psi-src").split("\n")) and page.is_hidden("#psi-ed-look"),
          "looks: the switch off takes the look line out")
    page.keyboard.press("Escape")
    # Values at their declared defaults stay out of links and paths.
    page.fill("#psi-src", "control k 0..1 = 0.5\ngray(k*x)")
    page.wait_for_timeout(400)
    f0 = page.evaluate("""() => import('./js/psikit.js').then((k) => { const c = { controls: [{ slot: 'a', default: 0.5 }] };
        return [k.path({ src: 'gray(a)', a: 0.5 }, c), k.path({ src: 'gray(a)', a: 0.25 }, c), k.vals({ src: 'gray(a)', a: 0.5 }, c), k.vals({ src: 'gray(a)', a: 0.75 }, c)]; })""")
    check(f0[0].startswith("@formula?psb=") and "&" not in f0[0] and f0[1] == f0[0] + "&a=0.25" and f0[2:] == ["", "&a=0.75"],
          f"looks: a value at its default stays out of paths and the site's hand-over ({f0})")
    put_json(base + "/api/config", {"look_formula": ""})
    urllib.request.urlopen(urllib.request.Request(base + "/api/files/formulas/matrix-glow.psi", method="DELETE"), timeout=30)
    page.context.close()


def psi_frames(page: Page, sel: str) -> int:
    return int(page.get_attribute(sel, "data-frames") or 0)


def formulas_combine_checks(browser, base: str, errors: list[str]) -> None:
    """`use <name> as p` puts an example in as `fn p(x, y) { … }` once the
    caret is off its line (the text stands alone), says plainly when there is
    no formula by that name, and the combined formula plays."""
    page = new_page(browser, "laptop", errors)
    page.goto(base + "/#formulas")
    formula_editor(page, "loop 8\nuse matrix as p\nlayer { p(1 - x, y) }")
    src = page.input_value("#psi-src")
    check("-- Matrix\nfn p(x, y) {\n" in src and "name \"Matrix\"" not in src and "loop 30" not in src and "use " not in src and page.get_attribute("#psi-ed", "data-err") is None
          and page.get_attribute("#psi-ed", "data-steps") is not None, f"formulas: use … as p puts the example in as a function ({src[:60]!r})")
    page.fill("#psi-src", "\nr")
    page.focus("#psi-src")
    page.keyboard.press("Control+Home")
    page.keyboard.type("use Big clock as r", delay=30)
    page.wait_for_timeout(400)
    typed, said = page.input_value("#psi-src"), page.inner_text("#psi-err")
    typing = typed == "use Big clock as r\nr" and "puts a formula in" in said
    page.keyboard.press("ArrowDown")
    page.wait_for_timeout(500)
    moved = "fn r(x, y) {" in page.input_value("#psi-src")
    check(typing and moved, f"formulas: a use line waits while the caret is on it ({typed!r}, {said!r}) and pastes when it moves off ({moved})")
    page.fill("#psi-src", "use No such thing as q\nq")
    page.wait_for_timeout(400)
    err = page.inner_text("#psi-err")
    page.wait_for_timeout(300)
    check(page.get_attribute("#psi-ed", "data-err") == "1" and "no formula called “No such thing”" in err
          and page.locator("#psi-err a.ecode").count() == 0,
          f"formulas: a use line naming nothing says so, with no code (the editor's own message) ({err!r})")
    mine = lambda: page.evaluate("JSON.parse(localStorage.getItem('odmd.psi.mine') || '[]').map(f => f.name)")
    before = mine()
    page.click("#psi-save")
    page.wait_for_timeout(400)
    toast = page.locator("#toasts .toast:has-text('First fix line 1')").count()
    check(toast >= 1 and mine() == before, f"formulas: Save refuses a use line it can't paste ({toast} toast, saved {mine()})")
    # A formula naming itself: never pasted into itself, not saved bare.
    page.fill("#psi-name", "Loopy")
    page.fill("#psi-src", "use Loopy as p\np")
    page.click("#psi-save")
    page.wait_for_timeout(400)
    err = page.inner_text("#psi-err")
    check("can’t be put inside itself" in err and "Loopy" not in mine() and page.input_value("#psi-src") == "use Loopy as p\np",
          f"formulas: a formula naming itself isn't pasted or saved ({err!r}, {mine()})")
    page.context.close()
    # A cycle in saved text (two formulas naming each other): a formula with
    # a use line of its own is no function, said so, no runaway.
    page = new_page(browser, "laptop", errors)
    page.goto(base + "/#formulas")
    page.evaluate("""localStorage.setItem('odmd.psi.mine', JSON.stringify([
        {id: 1, name: 'Ay', src: 'use Bee as q\\nq'}, {id: 2, name: 'Bee', src: 'use Ay as p\\np'}]))""")
    # (Copied onto the panel by the view, once: listed there before the hand-over.)
    page.evaluate("Object.keys(localStorage).filter(k => k.startsWith('odmd.psi.copied.')).forEach(k => localStorage.removeItem(k))")
    page.reload()
    page.wait_for_selector("#psi-mine .psi-tile[data-name='Bee']", timeout=20000)
    app = remix_open(page, base, errors, "use Bee as q\nq", "Ay")
    src, err = app.input_value("#psi-src"), app.inner_text("#psi-err")
    check("fn " not in src and "can’t be put in as a function" in err, f"formulas: a cycle through saved formulas is refused ({src!r}, {err!r})")
    page.context.close()


def formulas_more(browser, base: str, errors: list[str]) -> None:
    """Drafts not saved are kept (Continue editing, an in-page question before
    one is replaced); Formulas sent that try to inject markup or are too long; the
    drawing stops when nothing is shown; a trap in the module restarts it."""
    page = new_page(browser, "laptop", errors)
    page.goto(base + "/#formulas")
    formula_editor(page, "gray(x*0.25)")
    page.fill("#psi-name", "Smoke draft")
    page.wait_for_timeout(400)
    page.click("#psi-back")
    page.wait_for_selector("#psi-cont", timeout=5000)
    cont = page.inner_text("#psi-cont")
    page.click("#psi-ex .psi-tile >> nth=1")
    page.click("#psi-edit")
    page.wait_for_selector("#psi-ask", timeout=5000)
    page.click("#psi-ask button:has-text('Keep editing it')")
    page.wait_for_selector("#psi-ed #psi-src", timeout=5000)
    kept = page.input_value("#psi-src")
    page.click("#psi-back")
    page.wait_for_selector("#psi-cont", timeout=5000)
    page.click("#psi-edit")
    page.click("#psi-ask button:has-text('Replace it')")
    page.wait_for_function("document.querySelector('#psi-name')?.value.endsWith(' remix')", timeout=5000)
    page.click("#psi-back")
    page.wait_for_timeout(500)
    check("Smoke draft" in cont and kept == 'name "Smoke draft"\ngray(x*0.25)' and page.locator("#psi-cont").count() == 0,
          f"formulas: an unsaved draft is offered ('{cont}'), kept on Keep ({kept!r}) and replaced only on Replace")
    # The drawing stops once nothing shows (and starts again).
    page.click("#psi-ex .psi-tile >> nth=0")
    page.wait_for_timeout(300)
    goto_view(page, "settings")
    n0 = psi_frames(page, "#psi-panel")
    page.wait_for_timeout(600)
    n1 = psi_frames(page, "#psi-panel")
    idle = page.evaluate("new Promise(r => { let k = 0; const t = performance.now(); (function f() { if (++k < 30) requestAnimationFrame(f); else r(performance.now() - t); })(); })")
    goto_view(page, "formulas")
    page.wait_for_timeout(600)
    n2 = psi_frames(page, "#psi-panel")
    check(n0 == n1 and n2 > n1, f"formulas: no drawing while away ({n0} -> {n1}), drawing again on return ({n2}; rAF {idle:.0f} ms)")
    # A trap inside the module (forced: the glue's compile throws what a
    # trap throws) restarts it, and the editor carries on.
    page.context.close()
    page = new_page(browser, "laptop", errors)
    def trap_glue(route):
        r = route.fetch()
        body = r.text().replace("export function compile(src) {",
                                "export function compile(src) { if (globalThis.__psiTrap) { globalThis.__psiTrap = 0; throw new WebAssembly.RuntimeError('unreachable (smoke)'); }", 1)
        route.fulfill(response=r, body=body)
    page.route("**/pkg-psi/opendmd_psi_web.js*", trap_glue)
    page.goto(base + "/#formulas")
    formula_editor(page, "gray(x*0.5)")
    page.evaluate("globalThis.__psiTrap = 1")
    page.fill("#psi-src", "gray(x*0.75)")
    page.wait_for_selector("#toasts .toast:has-text('Formulas restarted')", timeout=10000)
    page.wait_for_timeout(500)
    page.fill("#psi-src", "loop 2\ngray(saw(x - t/2))")
    page.wait_for_timeout(600)
    f0 = psi_frames(page, "#psi-ed-panel")
    page.wait_for_timeout(400)
    check(page.get_attribute("#psi-ed", "data-err") is None and psi_frames(page, "#psi-ed-panel") > f0
          and page.get_attribute("#psi-ed", "data-steps") is not None, "formulas: a trap in the module restarts it and the editor carries on")
    page.context.close()
    # Hostile and over-long Formulas sent from the site: shown as text, cut
    # to size, nothing run; a message of the wrong shape ignored.
    page = new_page(browser, "laptop", errors)
    page.goto(base + "/version.txt")
    evil = '<img src=x onerror="window.__pwned=1">'
    app = remix_open(page, base, errors, "-- " + evil + "\ngray(x)", evil, bad_first=True)
    safe = (app.evaluate("window.__pwned") is None and app.locator("#psi-ed img, #psi-gal img").count() == 0
            and "img src=x" in app.input_value("#psi-name") and "-- <img" in app.input_value("#psi-src"))
    app.click("#psi-back")
    app.wait_for_selector("#psi-ex .psi-tile", timeout=5000)
    safe = safe and app.locator("#psi-gal img").count() == 0 and app.evaluate("window.__pwned") is None
    check(safe, "formulas: markup in a sent Formula's name and text stays text (the wrong shape first, ignored)")
    app.close()
    app = remix_open(page, base, errors, "gray(x)\n" + "-" * 17000, "N" * 200)
    src_len, name_len = len(app.input_value("#psi-src")), len(app.input_value("#psi-name"))
    check(src_len == 16384 and name_len == 60, f"formulas: an over-long Formula is cut ({src_len}, {name_len})")
    page.context.close()
    # (Both were saved as they arrived: gone again, for the checks after.)
    for e in get_json(base + "/api/files?dir=%2Fformulas")["entries"]:
        if e["name"].startswith(("img-src", "nnnn")):
            urllib.request.urlopen(urllib.request.Request(base + "/api/files/formulas/" + e["name"], method="DELETE"), timeout=30)


def formulas_phone_pass(browser, base: str, errors: list[str]) -> None:
    """At 360 px: no sideways scroll in the gallery, the Details sheet
    (Escape closes it) or the editor; a tap on a number brings the strip,
    whose + changes it."""
    ctx = browser.new_context(viewport={"width": 360, "height": 780}, device_scale_factor=3, is_mobile=True, has_touch=True)
    page = ctx.new_page()
    page.on("pageerror", lambda e: errors.append(f"[360] pageerror: {e}"))
    page.goto(base + "/#formulas")
    open_formulas(page)
    over = [page.evaluate("document.documentElement.scrollWidth - document.documentElement.clientWidth")]
    page.tap("#psi-ex .psi-tile >> nth=1")
    page.wait_for_selector("#psi-side.open", timeout=5000)
    over.append(page.evaluate("document.documentElement.scrollWidth - document.documentElement.clientWidth"))
    shot(page, "formulas-details", "phone")
    page.keyboard.press("Escape")
    check(page.is_hidden("#psi-side"), "formulas (360): Escape closes the Details sheet")
    page.tap("#psi-new")
    page.wait_for_selector("#psi-ed #psi-src", timeout=10000)
    page.wait_for_timeout(400)
    over.append(page.evaluate("document.documentElement.scrollWidth - document.documentElement.clientWidth"))
    check(over == [0, 0, 0], f"formulas (360): no sideways scroll (gallery, details, editor: {over})")
    shot(page, "formulas-editor", "phone")
    page.fill("#psi-src", "gray(x*0.5)")
    page.evaluate("(ta => { ta.focus(); ta.setSelectionRange(9, 9); })(document.querySelector('#psi-src'))")
    page.wait_for_selector("#psi-scrub:not([hidden])", timeout=5000)
    tall = page.evaluate("[...document.querySelectorAll('#psi-scrub > *')].every(e => e.getBoundingClientRect().height >= 44)")
    page.tap("#psi-scrub button[aria-label=More]")
    page.wait_for_timeout(300)
    check(page.input_value("#psi-src") == "gray(x*0.6)" and tall,
          f"formulas (360): the strip (44 px targets: {tall}) and its + ({page.input_value('#psi-src')!r})")
    ctx.close()


def formulas_demo_pass(browser, base: str, errors: list[str]) -> None:
    """The web demo (status `{}`, its card in memory): a Formula its site's
    page sends opens in the editor and plays."""
    page = new_page(browser, "laptop", errors)
    page.route("**/api/status", lambda r: r.fulfill(status=200, body="{}", headers={"content-type": "application/json"}))
    page.route("**/demo/index.json", lambda r: r.fulfill(status=200, body="[]", headers={"content-type": "application/json"}))
    page.goto(base + "/version.txt")
    page.context.route("**/api/status", lambda r: r.fulfill(status=200, body="{}", headers={"content-type": "application/json"}))
    page.context.route("**/demo/index.json", lambda r: r.fulfill(status=200, body="[]", headers={"content-type": "application/json"}))
    app = remix_open(page, base, errors, "loop 4\nrings = sin(x*4 - t/4)\nhsv(rings*0.5, 1, 1)", "Demo rings")
    app.wait_for_function("document.querySelector('#where').textContent.startsWith('Demo')", timeout=20000)
    wait_lit(app, "#psi-ed-panel", "formulas (demo): a Formula from the site's page")
    # (Saved in this browser as it arrived: its name is its name line now.)
    check(app.input_value("#psi-name") == "Demo rings" and "loop 4\nrings" in app.input_value("#psi-src")
          and app.locator("#psi-ed-show, #psi-show").count() == 0 and app.locator("#psi-ed-share").count() == 0,
          f"formulas (demo): it opened '{app.input_value('#psi-name')}', no Show on panel, no share link")
    page.context.close()


def recent_checks(page: Page, base: str) -> None:
    """Recently played and Previous on Now, against the mock's history
    (opendmd_core::history's rules): three GIFs played, Previous twice goes
    two back (no ping-pong) without reordering, a tap on an entry plays it
    and brings it to the front, Replay restarts the file on screen, and
    Previous stops at the oldest (404, the button disabled). Paused on the
    device for the length (preparing off), so its dwell cannot slip a file
    in."""
    def status(recent=True):
        return get_json(base + "/api/status" + ("?recent=1" if recent else ""))

    def back() -> int:
        req = urllib.request.Request(base + "/api/skip?back=1", data=b"", method="POST")
        try:
            with urllib.request.urlopen(req, timeout=30):
                return 200
        except urllib.error.HTTPError as e:
            return e.code

    def path_is(p):
        page.wait_for_function(f"document.querySelector('#now-path').textContent === {json.dumps(p)}", timeout=10000)

    def rows():
        return page.eval_on_selector_all("#now-recent > div", "els => els.map(e => [e.dataset.path, e.classList.contains('cur'), e.querySelector('.grow').textContent])")

    goto_view(page, "now")
    # Preparing off while paused, or the mock's simulated builder would run
    # (and leave an average the auto checks later do not expect).
    auto = get_json(base + "/api/config")["auto_prepare"]
    put_json(base + "/api/config", {"auto_prepare": "off"})
    post_json(base + "/api/pause", {})
    gifs = card_gifs()
    a, b, c = gifs[3], gifs[4], gifs[5]
    for g in (a, b, c):
        post_json(base + "/api/play", {"path": g})
    s = status()
    check("recent" not in status(False) and s["recent"][:3] == [c, b, a] and s["recent_at"] == 0
          and list(s).index("recent") == list(s).index("paused") + 1,
          f"recent: status?recent=1 lists the three plays newest first after `paused`, plain status has none ({s['recent'][:3]})")
    path_is(c)
    page.wait_for_function(f"document.querySelector('#now-recent > div')?.dataset.path === {json.dumps(c)}", timeout=10000)
    r = rows()
    title = page.inner_text("#now-title")
    check(r[0][1] and r[0][2] == title and not any(x[1] for x in r[1:]) and not page.is_hidden("#now-prev")
          and "/" not in "".join(x[2] for x in r),
          f"recent: the list shows titles like the Library ('{r[0][2]}' = '{title}'), the one on screen marked, Previous shown")
    page.click("#now-prev")
    path_is(b)
    page.wait_for_function(f"document.querySelector('#now-recent > div.cur')?.dataset.path === {json.dumps(b)}", timeout=10000)
    page.click("#now-prev")
    path_is(a)
    s = status()
    check(s["recent"][:3] == [c, b, a] and s["recent_at"] == 2 and s["now_playing"]["path"] == a,
          f"recent: Previous twice went two back without reordering (recent_at {s['recent_at']}, {s['now_playing']['path']})")
    page.wait_for_function(f"document.querySelector('#now-recent > div.cur')?.dataset.path === {json.dumps(a)}", timeout=10000)
    page.click(f"#now-recent > div[data-path={json.dumps(c)}]")
    path_is(c)
    s = status()
    check(s["recent"][:3] == [c, a, b] and s["recent_at"] == 0,
          f"recent: a tap plays the entry again, what was on screen just before it next ({s['recent'][:3]})")
    ordinal = s["now_playing"]["ordinal"]
    page.click("#now-again")
    page.wait_for_function(f"document.querySelector('#now-ord').textContent.startsWith('{ordinal + 1} ')", timeout=10000)
    s = status()
    check(s["now_playing"]["path"] == c and s["recent"][:3] == [c, a, b],
          f"recent: Replay restarts {c} (ordinal {ordinal} -> {s['now_playing']['ordinal']}), the list unchanged")
    codes = [back() for _ in range(RECENT_MAX + 1)]
    s = status()
    check(codes[-1] == 404 and 200 in codes and s["recent_at"] == len(s["recent"]) - 1,
          f"recent: Previous walks to the oldest of {len(s['recent'])}, then 404 ({codes.count(200)} steps)")
    page.wait_for_function("document.querySelector('#now-prev').disabled", timeout=10000)
    check(True, "recent: Previous disabled at the oldest")
    shot(page, "now-recent", "laptop")
    post_json(base + "/api/pause", {})
    put_json(base + "/api/config", {"auto_prepare": auto})


RECENT_MAX = 16


def now_edit_checks(page: Page, base: str) -> None:
    """Now's Edit, for the file on screen and for each recent entry: a GIF
    opens in Create Art from its original (the GIF, never its prepared
    copy), every frame, with its title; an effect's loop in Create Art; a
    Formula in the Formulas editor as it was saved (here an example: its
    name and comments); the clock says it is drawn by the panel, with no
    button."""
    gifs = card_gifs()
    a, b = gifs[6], gifs[7]
    asked: list[str] = []
    log = lambda r: r.method == "GET" and asked.append(urllib.parse.unquote(urllib.parse.urlsplit(r.url).path))
    page.on("request", log)

    def on_screen(p):
        post_json(base + "/api/play", {"path": p})
        goto_view(page, "now")
        # (A live item's path is not in #now-path: the recent list marks it.)
        page.wait_for_function(f"document.querySelector('#now-recent > div.cur')?.dataset.path === {json.dumps(p)}", timeout=10000)

    def studio_has(p):
        page.wait_for_selector("#view-create.on #studio .st-grid", timeout=20000)
        page.wait_for_function("(d => d.dataset.srcFrames && +d.dataset.srcFrames > 0)(document.querySelector('#studio'))", timeout=30000)
        return studio_data(page)

    on_screen(b)
    on_screen(a)
    check(page.inner_text("#now-edit") == "✎ Edit in Create Art" and page.inner_text("#now-live") == "",
          f"now edit: the GIF on screen offers '{page.inner_text('#now-edit')}'")
    asked.clear()
    page.click("#now-edit")
    d = studio_has(a)
    title = page.input_value("#st-title")
    copy = cache_path(a)
    check(a in asked and copy not in asked and title and page.is_visible("#st-mode [data-v=replace].on"),
          f"now edit: Create Art opens {a.rsplit('/', 1)[-1]} from the GIF, not its copy ({d.get('srcFrames')} frames, '{title}', Replace the original chosen)")
    goto_view(page, "now")
    page.wait_for_selector(f"#now-recent > div[data-path={json.dumps(b)}] button[aria-label='Edit in Create Art']", timeout=10000)
    asked.clear()
    # The studio still holds a's frames: clear the mark so studio_has waits for b's.
    page.evaluate("delete document.querySelector('#studio').dataset.srcFrames")
    page.click(f"#now-recent > div[data-path={json.dumps(b)}] button[aria-label='Edit in Create Art']")
    studio_has(b)
    s = get_json(base + "/api/status")
    check(b in asked and cache_path(b) not in asked and s["now_playing"]["path"] == a,
          f"now edit: a recent entry's ✎ opens {b.rsplit('/', 1)[-1]} in Create Art, without playing it")
    # A Formula: the example it is, in the editor.
    e = next(x for x in bundled() if x["name"] == "Old photo")
    path = page.evaluate("src => import('./js/looks.js').then(l => l.psi()).then(() => import('./js/psishow.js')).then(m => m.fpath({ src }))", e["src"])
    post_json(base + "/api/play", {"path": path})
    goto_view(page, "now")
    page.wait_for_function("document.querySelector('#now-edit')?.textContent === '✎ Edit in Formulas'", timeout=10000)
    check(page.inner_text("#now-edit") == "✎ Edit in Formulas", f"now edit: a Formula offers '{page.inner_text('#now-edit')}'")
    page.click("#now-edit")
    page.wait_for_selector("#view-formulas.on #psi-ed #psi-src", timeout=20000)
    if page.locator("button:has-text('Open it')").count():
        page.click("button:has-text('Open it')")
    page.wait_for_function(f"document.querySelector('#psi-src').value === {json.dumps(e['src'])}", timeout=10000)
    check(page.input_value("#psi-name") == "Old photo" and location_hash(page) == "#formulas/edit",
          "now edit: the Formula opens in the editor as the example it is, comments and all")
    # (A Formula sent from the editor is longer than an entry of Recently
    # played may be: the panel doesn't list it there.)
    # The clock: said, not offered.
    on_screen("@clock")
    page.wait_for_function("document.querySelector('#now-live').textContent !== ''", timeout=10000)
    clock_row = page.locator("#now-recent > div[data-path='@clock'] button[aria-label^='Edit']").count()
    check(page.is_hidden("#now-edit") and page.inner_text("#now-live") == "Clock: drawn by the panel itself, not a GIF" and clock_row == 0,
          f"now edit: the clock says '{page.inner_text('#now-live')}', no button")
    shot(page, "now-edit", "laptop")
    page.remove_listener("request", log)
    on_screen(a)


def location_hash(page: Page) -> str:
    return page.evaluate("location.hash")


def recent_demo_pass(browser, base: str, errors: list[str]) -> None:
    """Recently played in the web demo (local mode): a playlist of three
    live items, Next twice, Previous twice back to the first, a tap on an
    entry plays it."""
    page = new_page(browser, "laptop", errors)
    text = "@clock\n@text?msg=Mid\n@text?msg=Hi\n"
    page.route("**/api/status", lambda r: r.fulfill(status=200, body="{}", headers={"content-type": "application/json"}))
    page.route("**/demo/index.json", lambda r: r.fulfill(status=200, body=json.dumps([{"path": "/filelist.txt", "size": len(text)}]),
                                                     headers={"content-type": "application/json"}))
    page.route("**/demo/filelist.txt", lambda r: r.fulfill(status=200, body=text, headers={"content-type": "text/plain"}))
    page.goto(base + "/")
    page.wait_for_function("document.querySelector('#now-title')?.textContent === 'Clock'", timeout=20000)
    titles = lambda: page.eval_on_selector_all("#now-recent > div", "els => els.map(e => (e.classList.contains('cur') ? '*' : '') + e.querySelector('.grow').textContent)")
    page.click("#now-skip")
    page.click("#now-skip")
    page.wait_for_function("document.querySelector('#now-title').textContent === 'Hi'", timeout=10000)
    t1 = titles()
    page.click("#now-prev")
    page.click("#now-prev")
    page.wait_for_function("document.querySelector('#now-title').textContent === 'Clock'", timeout=10000)
    t2 = titles()
    check(t1 == ["*Hi", "Mid", "Clock"] and t2 == ["Hi", "Mid", "*Clock"] and page.is_disabled("#now-prev"),
          f"recent (demo): Next twice {t1}, Previous twice {t2}, Previous disabled at the oldest")
    page.click("#now-recent > div:nth-child(2)")
    page.wait_for_function("document.querySelector('#now-title').textContent === 'Mid'", timeout=10000)
    t3 = titles()
    check(t3 == ["*Mid", "Clock", "Hi"], f"recent (demo): a tap plays the entry, the one on screen before it next ({t3})")
    page.context.close()


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
    page.route(SITE_ART + "SNES.json", lambda r: r.fulfill(status=200, body=json.dumps(STUB_INDEX),
                                                           headers={"access-control-allow-origin": "*", "content-type": "application/json"}))
    page.route("https://raw.githubusercontent.com/**", raw)


def open_studio(page: Page, via: str = "#nav-art") -> None:
    """Create Art from the nav (`#nav-art`), or its old address `#studio`."""
    if via.startswith("#nav"):
        page.click(via)
    else:
        page.evaluate(f"location.hash = '{via}'")
    page.wait_for_selector("#view-create.on #studio .st-grid", timeout=20000)
    check(page.url.endswith("#create"), f"studio: {via} opens the Create Art view ({page.url.rsplit('/', 1)[-1]})")


def studio_data(page: Page) -> dict:
    return page.evaluate("({...document.querySelector('#studio').dataset})")


def studio_shots(page: Page, size: str) -> None:
    page.evaluate("scrollTo(0, 0)")
    shot(page, "studio", size)
    page.evaluate("scrollTo(0, 1e6)")
    page.wait_for_timeout(200)
    shot(page, "studio-save", size)
    page.evaluate("scrollTo(0, 0)")


def studio_boxes(page: Page) -> list[list[float]]:
    """[x, y, width] of the six steps, in order 1-6."""
    return [page.eval_on_selector(f"#st-{x}", "e => { const r = e.getBoundingClientRect(); return [r.x, r.y, r.width]; }")
            for x in ("import", "frame", "look", "text-step", "preview", "save-step")]


def studio_checks(page: Page, base: str, overlay: Path) -> None:
    fixture = studio_fixture()
    hits: list[str] = []
    stub_art(page, hits)
    open_studio(page, "#studio")
    # (A picture from an earlier check folds Find: open it again.)
    if page.is_visible("#st-change"):
        page.click("#st-change")
    # Find's Game art: the index is not in the pack, it is the site's.
    try:
        urllib.request.urlopen(base + "/art/SNES.json")
        packed = True
    except urllib.error.HTTPError as e:
        packed = e.code != 404
    check(not packed, "studio find: the art index is not served from the player (/art/SNES.json 404)")
    # The site unreachable: a sentence and Try again; then the real N64 index
    # from the site's stand-in, fetched once for two searches.
    n64 = SITE_ART + "N64.json"
    expected.append(n64)
    page.route(n64, lambda r: r.abort("internetdisconnected"))
    page.click("#st-src [data-v=gameart]")
    page.select_option("#find-platform", "N64")
    page.fill("#st-q", "mario")
    page.click("#st-go")
    page.wait_for_selector("#find-status #find-retry", timeout=20000)
    t = page.inner_text("#find-status")
    check(t.startswith("Game art needs the internet") and page.locator("#find-grid .find-hit").count() == 0,
          f"studio find: the site unreachable -> '{t}'")
    page.unroute(n64)
    before = len(site_art_asked)
    page.click("#find-retry")
    page.wait_for_selector("#find-grid .find-hit", timeout=20000)
    names = page.eval_on_selector_all("#find-grid .find-hit .find-t", "els => els.map(e => e.textContent)")
    got = site_art_asked[before:]
    check(bool(names) and all("mario" in n.lower() for n in names) and [u for u, _ in got] == [n64] and all(o for _, o in got),
          f"studio find: Try again -> N64 'mario' from the site's index, asked with CORS ({len(names)} tiles, e.g. {names[:1]}; {got})")
    page.fill("#st-q", "zelda")
    page.click("#st-go")
    page.wait_for_function("[...document.querySelectorAll('#find-grid .find-t')].some(e => e.textContent.toLowerCase().includes('zelda'))", timeout=20000)
    check(len(site_art_asked) == before + 1, f"studio find: a second search uses the index already fetched ({len(site_art_asked) - before} fetches)")
    expected.remove(n64)
    # The stubbed SNES index, then a game's pictures.
    page.fill("#st-q", "")
    page.select_option("#find-platform", "SNES")
    page.fill("#st-q", "studio test")
    page.click("#st-go")
    # The pictures of the games found, best game first, as the find.js tile grid.
    page.wait_for_selector("#find-grid .find-hit")
    tiles = page.eval_on_selector_all("#find-grid .find-hit", "els => els.map(e => [e.dataset.kind, e.innerText.replace(/\\n+/g, ' / ')])")
    check(tiles == [["Named_Titles", "Studio Test (USA) / Title screen"], ["Named_Snaps", "Studio Test (USA) / In the game"],
                    ["Named_Snaps", "Studio Test (Japan) / In the game"]], f"studio find: 'studio test' on SNES -> tiles {tiles}")
    page.wait_for_function("[...document.querySelectorAll('#find-grid .find-hit img')].filter(i => i.complete && i.naturalWidth).length === 3", timeout=10000)
    linked = any(h.endswith("Named_Snaps/Studio Test (Japan).png") for h in hits)
    check(linked and page.locator("#studio .st-hit, #studio .st-thumb").count() == 0,
          f"studio find: every picture loads, the USA snap through its link ({len(hits)} requests)")
    # (The view keeps its picture between visits: forget the last one's frames.)
    page.evaluate("delete document.querySelector('#studio').dataset.frames")
    page.click("#find-grid .find-hit[data-kind=Named_Titles]")
    page.wait_for_function("document.querySelector('#studio').dataset.frames", timeout=20000)
    # (The name fields land after the frames: naming() awaits the category's listing.)
    try:
        page.wait_for_function("document.querySelector('#studio').dataset.title === 'Studio Test'", timeout=5000)
    except Exception:
        pass
    d = studio_data(page)
    check(d.get("platform") == "SNES" and d.get("title") == "Studio Test",
          f"studio find: a click on a picture loads it, platform and title filled in ({d.get('platform')}, {d.get('title')})")
    # Loaded: Find folds to one line; laptop: Frame left, Look + Text right,
    # Preview and Save full width; the steps headed "1 Find" ...; Save the
    # view's one primary.
    check(not page.is_visible("#find-grid") and page.is_visible("#st-change") and page.is_visible("#st-frame"),
          "studio find: Find folds, Change picture shown, the steps appear")
    imp, frm, look, txt, prv, sv = studio_boxes(page)
    check(imp[2] == prv[2] == sv[2] > 2 * frm[2] and frm[0] < look[0] == txt[0] and frm[1] == look[1] > imp[1]
          and txt[1] > look[1] and prv[1] > max(frm[1], txt[1]) and sv[1] > prv[1],
          f"studio layout: Find, two columns, then Preview and Save ({[b[:2] for b in (imp, frm, look, txt, prv, sv)]})")
    heads = page.eval_on_selector_all("#studio .card > h2", "els => els.map(e => e.textContent)")
    pri = page.eval_on_selector_all("#studio button.pri", "els => els.map(e => e.id)")
    check(heads == ["1 Find", "2 Frame", "3 Look", "4 Text", "5 Preview", "6 Save"] and pri == ["st-save"],
          f"studio: step headings {heads}, primary {pri}")
    page.click("#st-change")
    check(page.is_visible("#find-grid") and page.locator("#find-grid .find-hit").count() == 3 and page.input_value("#st-q") == "studio test",
          "studio find: Change picture opens Find again, its results as they were")
    # The same picture from a file: the upscale undone, Pixels, Pan.
    page.set_input_files("#st-file", str(fixture))
    page.wait_for_function("document.querySelector('#st-msg').textContent.startsWith('Picture 768')", timeout=20000)
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
    # An address whose site refuses the browser (no CORS): the panel's app has
    # no relay (the site's Create Art has), so it says what to do instead.
    page.route("https://refuses.example/**", lambda r: r.abort())
    expected.append("refuses.example")  # (the refused load's console line is the point here)
    page.click("#st-change")
    page.click("#st-src [data-v=own]")
    page.fill("#st-url", "https://refuses.example/a.gif")
    page.click("#st-url-go")
    page.wait_for_function("document.querySelector('#st-msg').textContent.includes('download')", timeout=10000)
    check(page.inner_text("#st-msg") == "That site doesn’t allow it: download the picture, then add the file here",
          f"studio address: a site that refuses the browser is said, with what to do ('{page.inner_text('#st-msg')}')")
    page.unroute("https://refuses.example/**")
    expected.remove("refuses.example")
    # A video: the panel's app has no video reader (the site's Create Art has).
    page.set_input_files("#st-file", str(REPO / "tools/fixtures/video/clip.webm"))
    page.wait_for_function("document.querySelector('#st-msg').textContent.startsWith('That is a video')", timeout=10000)
    check(page.inner_text("#st-msg") == "That is a video: Create Art on opendmd.dev makes videos into GIFs; here, pick a GIF or a picture"
          and page.get_attribute("#st-file", "accept") == "image/*",
          f"studio video: the panel's app says where videos can be made into GIFs ('{page.inner_text('#st-msg')}')")
    page.click("#st-src [data-v=gameart]")
    page.click("#st-change")
    goto_view(page, "library")
    page.unroute(SITE_ART + "SNES.json")
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
    """An animated GIF in Create Art: a 12-frame fixture loads with its frame
    count shown and the preview advancing; trim to frames 3-10, speed 2x,
    delete frame 6 (Undo, then again), text over it; saved, the GIF decodes
    to the 7 frames expected with 30 ms delays and the text, its prepared
    copy is in place and the Library lists it; Edit in Create Art brings it
    back whole, Reverse and Replace the original rewrite the same file."""
    goto_view(page, "create")
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
    def cell(i: int) -> str:
        """Frame i's cell, the strip scrolled to it (only the cells in view exist)."""
        page.eval_on_selector("#st-strip", "(s, i) => { s.scrollLeft = Math.max(0, i * 136 - 200); s.dispatchEvent(new Event('scroll')); }", i)
        sel = f"#st-strip .st-cell[data-i='{i}']:not([hidden])"
        page.wait_for_selector(sel, timeout=5000)
        return sel
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
    page.select_option("#st-plat", "")
    ph0 = page.get_attribute("#st-author", "placeholder")
    page.select_option("#st-plat", "SNES")
    ph1 = page.get_attribute("#st-author", "placeholder")
    check("needs a platform" in ph0 and ph1 == "optional", f"studio: Author says it needs a platform until one is picked ({ph0!r} -> {ph1!r})")
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
    # Edit in Create Art: the whole animation back; reversed, the original replaced.
    page.click("#lib-d-art")
    page.wait_for_function("(d => d.dataset.animated && d.dataset.srcFrames === '7')(document.querySelector('#studio'))", timeout=30000)
    check(page.input_value("#st-title") == "Smoke Frames" and page.is_visible("#st-mode [data-v=replace].on"),
          "studio anim: Edit in Create Art brings the 7 frames, its title, Replace the original chosen")
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
    goto_view(page, "create")
    studio_timings(page, "mock")
    goto_view(page, "library")


def scroll_fixture(path: Path, n: int = 16, step: int = 4) -> None:
    """An n-frame 128x32 GIF of a random block pattern scrolling `step` px
    left a frame, looping seamlessly: every pair a shift Smooth motion takes
    (opendmd-pack smooth analyse: 16 of 16 pairs, +32 frames)."""
    import random
    from PIL import Image, ImageDraw
    rnd = random.Random(7)
    period = n * step
    pal = [(250, 210, 30), (230, 60, 40), (40, 120, 230), (30, 30, 30), (240, 240, 240), (60, 200, 90)]
    tile = Image.new("RGB", (period, 32), (10, 10, 40))
    d = ImageDraw.Draw(tile)
    for _ in range(40):
        x, y, w, h = rnd.randrange(period), rnd.randrange(32), rnd.randrange(3, 10), rnd.randrange(3, 10)
        d.rectangle([x, y, x + w, y + h], fill=rnd.choice(pal))
    strip = Image.new("RGB", (128 + period * 2, 32))
    for k in range(0, strip.width, period):
        strip.paste(tile, (k, 0))
    frames = [strip.crop((i * step, 0, i * step + 128, 32)).convert("P", palette=Image.Palette.ADAPTIVE, colors=16) for i in range(n)]
    frames[0].save(path, save_all=True, append_images=frames[1:], duration=60, loop=0, optimize=False)


def pack_ref(gif_file: Path, path: str, smooth: bool) -> bytes:
    """opendmd-pack convert's copy of a GIF saved at `path`, listed in
    /sets/.smooth or not, on a card of its own."""
    mini = OUT / "pack-studio-smooth"
    shutil.rmtree(mini, ignore_errors=True)
    dst = mini / path.lstrip("/")
    dst.parent.mkdir(parents=True)
    shutil.copy(gif_file, dst)
    if smooth:
        (mini / "sets").mkdir()
        (mini / "sets" / ".smooth").write_text(path + "\n")
    pack("convert", str(mini), "--verify")
    return (mini / cache_path(path).lstrip("/")).read_bytes()


def studio_smooth_checks(page: Page, base: str, overlay: Path) -> None:
    """Smooth motion in Create Art: the switch analyses the output, the
    preview plays it smoothed, Save lists it and writes the smooth copy;
    Replace the original keeps it, or drops it when switched off."""
    listing = overlay / "sets" / ".smooth"
    before = listing.read_text() if listing.exists() else None
    listed = lambda: [l for l in (listing.read_text() if listing.exists() else "").split("\n") if l]
    smooth_state = "document.querySelector('#studio').dataset.smooth"
    settled = f"(s => s && s !== 'analysing')({smooth_state})"
    goto_view(page, "create")
    # Nothing to smooth: the 12 flat-colour frames of studio_anim_checks.
    page.evaluate("delete document.querySelector('#studio').dataset.tImport")
    page.set_input_files("#st-file", str(OUT / "anim-fixture.gif"))
    page.wait_for_function("document.querySelector('#studio').dataset.tImport", timeout=30000)
    check(page.is_visible("#st-smooth") and not page.is_checked("#st-smooth"), "studio smooth: the switch shows for an animation, off")
    page.check("#st-smooth")
    page.wait_for_function(settled, timeout=60000)
    st, cap = page.evaluate(smooth_state), page.inner_text("#st-smooth-cap")
    check(st == "none" and page.is_checked("#st-smooth") and cap == "Nothing here slides across the screen in steps it can smooth",
          f"studio smooth: nothing to smooth -> on, but says so: '{cap}' ({st})")
    # A scroll: on -> analysed, the preview plays the smoothed frames.
    fx = OUT / "scroll-fixture.gif"
    scroll_fixture(fx)
    page.evaluate("delete document.querySelector('#studio').dataset.tImport")
    page.set_input_files("#st-file", str(fx))
    page.wait_for_function("document.querySelector('#studio').dataset.srcFrames === '16' && document.querySelector('#studio').dataset.tImport", timeout=30000)
    check(page.evaluate(smooth_state) == "off" and not page.is_checked("#st-smooth"), "studio smooth: a new GIF starts off")
    page.check("#st-smooth")
    page.wait_for_function(settled, timeout=60000)
    cap = page.inner_text("#st-smooth-cap")
    m = re.fullmatch(r"Adds (\d+) frames? across (\d+) moves?", cap)
    check(page.evaluate(smooth_state) == "on" and m is not None and int(m[1]) > 0, f"studio smooth: on -> '{cap}'")
    page.wait_for_function("document.querySelector('#studio').dataset.shown === 'smooth'", timeout=10000)
    wait_lit(page, "#st-panel", "studio smooth preview")
    check(True, "studio smooth: the preview plays the smoothed frames")
    # An edit makes the draft again.
    page.select_option("#st-rate", "200")
    check(page.evaluate(smooth_state) == "analysing", "studio smooth: a speed change looks again")
    page.wait_for_function(settled, timeout=60000)
    print(f"  studio smooth at 2x: '{page.inner_text('#st-smooth-cap')}'")
    page.select_option("#st-rate", "100")
    page.wait_for_function(settled, timeout=60000)
    check(page.inner_text("#st-smooth-cap") == cap, f"studio smooth: back at 1x, the same answer ('{page.inner_text('#st-smooth-cap')}')")
    shot(page, "studio-smooth", "laptop")
    # Save as new: listed, the copy smooth.
    page.select_option("#st-plat", "SNES")
    page.fill("#st-author", "")
    page.fill("#st-title", "Smoke Smooth")
    name = "SNES_Smoke_Smooth_01.gif"
    page.wait_for_function(f"document.querySelector('#studio').dataset.name === '{name}'", timeout=10000)
    page.evaluate("delete document.querySelector('#studio').dataset.saved")
    page.click("#st-save")
    page.wait_for_function("document.querySelector('#studio').dataset.saved", timeout=60000)
    path = f"/gifs/Consoles/{name}"
    got, copy = overlay / path.lstrip("/"), overlay / cache_path(path).lstrip("/")
    c = copy.read_bytes() if copy.exists() else b""
    check(path in listed() and marked(c), f"studio smooth: saved as new -> listed ({listed()}), the copy marked ({len(c)} B)")
    check(c == pack_ref(got, path, True), "studio smooth: the copy byte-identical to opendmd-pack's with the list")
    lib_row(page, path)
    page.wait_for_function("(s => s && s !== 'analysing')(document.querySelector('#lib-smooth').dataset.state)", timeout=60000)
    check(page.get_attribute("#lib-smooth", "data-state") == "on", "studio smooth: Details shows it on")
    # Edit in Create Art: on from the start; reversed, the original replaced.
    page.click("#lib-d-art")
    page.wait_for_selector("#st-mode [data-v=replace].on", state="visible", timeout=30000)
    page.wait_for_function(f"{smooth_state} === 'on'", timeout=60000)
    check(page.is_checked("#st-smooth") and page.evaluate(smooth_state) == "on" and page.is_visible("#st-mode [data-v=replace].on"),
          f"studio smooth: a listed GIF opens with the switch on ('{page.inner_text('#st-smooth-cap')}')")
    page.check("#st-rev")
    page.wait_for_function(settled, timeout=60000)
    page.evaluate("delete document.querySelector('#studio').dataset.saved")
    page.click("#st-save")
    page.wait_for_function("document.querySelector('#studio').dataset.saved", timeout=60000)
    c = copy.read_bytes()
    check(listed().count(path) == 1 and marked(c) and c == pack_ref(got, path, True),
          f"studio smooth: Replace the original, left on -> still listed, the copy smooth ({len(c)} B)")
    # Off, replaced: the plain copy, unlisted.
    page.uncheck("#st-smooth")
    check(page.evaluate(smooth_state) == "off", "studio smooth: switched off")
    page.evaluate("delete document.querySelector('#studio').dataset.saved")
    page.click("#st-save")
    page.wait_for_function("document.querySelector('#studio').dataset.saved", timeout=60000)
    c = copy.read_bytes()
    check(path not in listed() and not marked(c) and c == pack_ref(got, path, False),
          f"studio smooth: off and replaced -> unlisted ({listed()}), the plain copy ({len(c)} B)")
    lib_row(page, path)
    page.wait_for_function("(s => s && s !== 'analysing')(document.querySelector('#lib-smooth').dataset.state)", timeout=60000)
    check(page.get_attribute("#lib-smooth", "data-state") == "off", "studio smooth: Details shows it off")
    no_leaks(page, "studio (smooth)")
    # Leave /sets/.smooth as it was for the Smooth motion checks.
    if before is None:
        listing.unlink(missing_ok=True)
    else:
        listing.write_text(before)


# --- The Library's web sources (js/find.js) ------------------------------------

GC_HITS = [  # GifCities' JSON (gifcities.archive.org/api/v1/gifsearch): three shapes
    {"url_text": "spinning  mario 2", "weight": 90, "gif": "x/a.gif", "checksum": "SQUARECHECKSUM", "height": 64, "width": 64,
     "page": "https://web.archive.org/web/2009/http://geocities.com/a/index.html"},
    {"url_text": "cool mario banner 1", "weight": 50, "gif": "x/b.gif", "checksum": "WIDECHECKSUM", "height": 31, "width": 88,
     "page": "https://web.archive.org/web/2009/http://geocities.com/b/index.html"},
    {"url_text": "mario tower", "weight": 20, "gif": "x/c.gif", "checksum": "TALLCHECKSUM", "height": 120, "width": 50,
     "page": "https://web.archive.org/web/2009/http://geocities.com/c/index.html"},
]


# blob.gifcities.org (SeaweedFS) answers with a CORS header only when the
# request carries Origin. A plain <img> sends none, so if the tiles were
# plain images Chromium's cache would hand the later fetch() of the bytes a
# no-CORS entry and it would fail (seen on the rig, 2026-10-01); the tiles
# load with crossOrigin so the cache entry serves both.
def gc_blob_headers(request):
    h = {"content-type": "image/gif", "cache-control": "max-age=3600"}
    if request.headers.get("origin"):
        h["access-control-allow-origin"] = "*"
    return h

def find_checks(page: Page) -> None:
    """Create Art -> GifCities against a canned answer and a 2-frame 88x31
    GIF (Playwright routes; no network): the search, the count, the Wide chip
    narrowing to the one wide hit, a click loading it (2 frames, the title
    hint in Save, "Seen on" the page it was on, Find folded); Change picture;
    a failed query's sentence. The Library: no web sources, and a search that
    finds nothing there offers the web: the site's Create Art in a new tab,
    its words (and the one platform picked) in the address."""
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
    origins: list[str | None] = []

    def blob(route) -> None:
        origins.append(route.request.headers.get("origin"))
        route.fulfill(status=200, body=gif.read_bytes(), headers=gc_blob_headers(route.request))
    page.route("https://blob.gifcities.org/**", blob)
    # Game art as Create Art's source, then the Library: only what is on
    # the player; nothing found -> the web (GifCities, no platform chip on).
    goto_view(page, "create")
    if page.is_visible("#st-change") and page.inner_text("#st-change") == "Change picture":
        page.click("#st-change")
    page.click("#st-src [data-v=gameart]")
    goto_view(page, "library")
    pick_cat(page, "Consoles")
    page.wait_for_function("document.querySelector('#lib-count').textContent.includes(' of ')", timeout=60000)
    check(page.locator("#lib-src, #lib-go, #lib-art").count() == 0, "find: the Library has no web sources")
    page.fill("#lib-search", "mario")
    page.wait_for_timeout(300)
    check(not page.is_visible("#lib-web"), "find: a search that finds GIFs offers no web search")
    page.fill("#lib-search", "zzqx mario banner")
    page.wait_for_selector("#lib-web:not([hidden])")
    check("“zzqx mario banner”" in page.inner_text("#lib-web"), f"find: nothing here -> '{page.inner_text('#lib-web')}'")
    check(not page.is_visible("#view-library .vlist"), "find: no empty list under the hint")
    # Search the web: the site's Create Art, in a new tab, with the words (its web sources are the site's).
    page.context.route("https://opendmd.dev/create/**", lambda r: r.fulfill(status=200, body="<p>create</p>", content_type="text/html"))
    with page.context.expect_page() as info:
        page.click("#lib-web-go")
    tab = info.value
    tab.wait_for_load_state()
    check(tab.url == "https://opendmd.dev/create/?q=zzqx+mario+banner" and "#library" in page.url,
          f"find: Search the web opens the site's Create Art with the words ({tab.url})")
    tab.close()
    goto_view(page, "create")
    page.click("#st-src [data-v=gifcities]")
    check(page.get_attribute("#st-q", "placeholder") == "Search GifCities", "find: the box searches GifCities")
    asked.clear()
    page.fill("#st-q", "mario")
    page.press("#st-q", "Enter")
    page.wait_for_function("document.querySelector('#find-count').textContent === '3 of 3'", timeout=10000)
    check(len(asked) == 1 and "q=mario" in asked[0] and "limit=300" in asked[0], f"find: one query, 300 asked for ({asked})")
    dims = page.eval_on_selector_all("#find-grid .find-hit small", "els => els.map(e => e.textContent)")
    check(dims == ["64×64", "88×31", "50×120"], f"find: 3 of 3, best match first, sizes shown ({dims})")
    page.wait_for_function("[...document.querySelectorAll('#find-grid img')].every(i => i.complete && i.naturalWidth === 88)", timeout=10000)
    # Routed answers never enter Chromium's cache, so the rig's failure (a
    # plain <img> entry served to fetch()) cannot be reproduced here: check
    # its cause instead — every tile asked with Origin (crossOrigin set).
    check(len(origins) >= 3 and all(origins), f"find: the tiles load with CORS (Origin sent on {sum(1 for o in origins if o)} of {len(origins)})")
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
    check("Results are unfiltered." in page.inner_text("#find-caption"), "find: the caption says results are unfiltered")
    no_leaks(page, "create art (GifCities)")
    shot(page, "create-find", "laptop")
    page.evaluate("delete document.querySelector('#studio').dataset.tImport")
    page.click("#find-grid .find-hit[data-id=WIDECHECKSUM]")
    page.wait_for_function("(d => d && d.dataset.tImport && d.dataset.srcFrames === '2')(document.querySelector('#studio'))", timeout=30000)
    msg = page.inner_text("#st-msg")
    check(msg.startswith("Cool Mario Banner 1: 88 × 31 · 2 frames") and page.input_value("#st-title") == "Cool Mario Banner",
          f"find: a click loads it ({msg!r}, title {page.input_value('#st-title')!r})")
    check(page.get_attribute("#st-seen", "href") == GC_HITS[1]["page"] and page.is_visible("#st-seen")
          and not page.is_visible("#find-grid") and page.is_visible("#st-change"),
          "find: Find folds to one line, 'Seen on' its page")
    page.click("#st-change")
    check(page.is_visible("#find-grid") and page.input_value("#st-q") == "mario", "find: Change picture brings the results back")
    page.click("#st-change")
    check(not page.is_visible("#find-grid") and page.inner_text("#st-change") == "Change picture", "find: Keep this picture folds Find again")
    page.click("#st-change")
    # A failed query: a sentence.
    expected.append("gifcities.archive.org")
    page.unroute("https://gifcities.archive.org/api/v1/gifsearch*")
    page.route("https://gifcities.archive.org/api/v1/gifsearch*", lambda r: r.abort())
    page.fill("#st-q", "luigi")
    page.click("#st-go")
    page.wait_for_function("document.querySelector('#find-status').textContent.includes('didn’t answer')", timeout=10000)
    check(page.inner_text("#find-count") == "", f"find: no answer -> '{page.inner_text('#find-status')}'")
    page.click("#st-src [data-v=gameart]")
    page.click("#st-src [data-v=gifcities]")
    # One platform chip on: the hand-off searches that platform's game art.
    goto_view(page, "library")
    page.fill("#lib-search", "")
    page.click("#lib-chips [data-platform='SNES']")
    page.fill("#lib-search", "zzqx harvest moon")
    page.wait_for_selector("#lib-web:not([hidden])")
    with page.context.expect_page() as info:
        page.click("#lib-web-go")
    tab = info.value
    tab.wait_for_load_state()
    check(tab.url == "https://opendmd.dev/create/?q=zzqx+harvest+moon&platform=SNES",
          f"find: with SNES picked, Search the web names the platform ({tab.url})")
    tab.close()
    page.context.unroute("https://opendmd.dev/create/**")
    page.fill("#lib-search", "")
    page.click("#lib-chips [data-platform='SNES']")
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
    page.wait_for_function(f"(document.querySelector('#lib-cap').dataset.shown || '') === {json.dumps(path.rsplit('/', 1)[1])}", timeout=30000)


def details_checks(page: Page, base: str, overlay: Path) -> None:
    """The Details panel of a known GIF; Add GIFs' dialog (a title guessed
    from a name in the grammar, none from a hash-like one, which must be
    given one); Edit details renaming it through POST /api/files/move, its
    prepared copy, playlist line, favourite and Smooth motion line
    following; Edit in Create Art; Delete taking all of it."""
    move_api_checks(base)
    known = "/gifs/Consoles/SNES_Mega_Man_X_08_sephirot68.gif"
    lib_row(page, known)
    got = {k: page.inner_text(f"#lib-d-{k}") for k in ("title", "platform", "category", "author")}
    check(got == {"title": "Mega Man X 08", "platform": "SNES", "category": "Consoles", "author": "sephirot68"},
          f"details: the fields of {known.rsplit('/', 1)[1]} ({got})")
    meta = page.inner_text("#lib-d-meta")
    check(meta == "SNES · Consoles · by sephirot68", f"details: one line '{meta}'")
    for sel in ("#lib-play", "#lib-d-add", "#lib-d-art", "#lib-d-more"):
        check(page.is_visible(sel), f"details: {sel} shown")
    page.click("#lib-d-more")
    for sel in ("#lib-d-fav", "#lib-d-hide", "#lib-d-edit", "#lib-d-del"):
        check(page.is_visible(sel), f"details: {sel} in the … menu")
    check("danger" in (page.get_attribute("#lib-d-del", "class") or ""), "details: Delete… is destructive in the menu")
    page.keyboard.press("Escape")
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
    add_files(page, [named, hashy])
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
    details_do(page, "#lib-d-fav")
    page.click("#lib-in-menu [data-path='/sets/cachetest.txt']")
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
    details_do(page, "#lib-d-edit")
    page.wait_for_selector("#dl .dl-item[data-i='0'][data-name]", timeout=20000)
    f = {k: page.input_value(item(0, f".dl-{k}")) for k in ("title", "plat", "cat", "author")}
    check(f == {"title": "Smoke Test Card", "plat": "OTHER", "cat": "Consoles", "author": "smoker"}, f"edit: the fields read back ({f})")
    page.fill(item(0, ".dl-title"), "Smoke Renamed")
    page.select_option(item(0, ".dl-cat"), "Other")
    to = "/gifs/Other/OTHER_Smoke_Renamed_01_smoker.gif"
    page.wait_for_function(f"document.querySelector('#dl .dl-item[data-i=\"0\"]').dataset.name === {json.dumps(to.rsplit('/', 1)[1])}", timeout=10000)
    shot(page, "library-edit-dialog", "laptop")
    page.click("#dl-ok")
    page.wait_for_function(f"(document.querySelector('#lib-cap').dataset.shown || '') === {json.dumps(to.rsplit('/', 1)[1])}", timeout=60000)
    check(not (overlay / card.lstrip("/")).exists() and (overlay / to.lstrip("/")).exists(), f"edit: renamed on the card to {to}")
    check(not (overlay / cache_path(card).lstrip("/")).exists() and (overlay / cache_path(to).lstrip("/")).exists(),
          f"edit: the prepared copy moved to {cache_path(to)}")
    lines_now = plist_file.read_text().split("\n")
    check(to in lines_now and card not in lines_now, "edit: the playlist line follows")
    check(meta.read_text().startswith(to + "\t"), "edit: the favourite follows")
    check(smooth.read_text().split("\n").count(to) == 1 and card not in smooth.read_text(), "edit: the Smooth motion line follows")
    check(page.inner_text("#lib-d-title") == "Smoke Renamed 01" and page.inner_text("#lib-d-category") == "Other",
          f"edit: the Details show the new fields ({page.inner_text('#lib-d-title')}, {page.inner_text('#lib-d-category')})")
    # Edit in Create Art: the GIF — every frame — is the Studio's source.
    from PIL import Image
    want = Image.open(small).n_frames
    page.click("#lib-d-art")
    # (Waits for this GIF's frames: the Studio may still hold what Now's Edit opened.)
    try:
        page.wait_for_function(f"(d => d && d.dataset.animated && d.dataset.srcFrames === '{want}')(document.querySelector('#studio'))", timeout=30000)
    except Exception:
        pass
    got = int(studio_data(page)["srcFrames"])
    check(got == want and f"{want} frames" in page.inner_text("#st-msg"),
          f"edit in make art: the whole animation loads ({got} frames, the file has {want}; '{page.inner_text('#st-msg')}')")
    check(page.input_value("#st-title") == "Smoke Renamed" and page.input_value("#st-plat") == "OTHER"
          and page.input_value("#st-author") == "smoker" and page.is_visible("#st-mode"),
          f"edit in make art: loaded with its fields ({page.input_value('#st-title')}, {page.input_value('#st-plat')}, "
          f"{page.input_value('#st-author')}) and Replace / Save as new")
    # Delete: after asking, the GIF, its copy, its playlist line, favourite, Smooth line.
    lib_row(page, to)
    # The thumbnails drawn and none being read: a row's still asked for the
    # renamed GIF (its copy first, then the file) would otherwise be answered
    # after the delete, by two 404s that are the app's right answer and not
    # a page error. (On a loaded machine the read was still queued.)
    wait_thumbs_idle(page)
    # (A preview's read begun before the delete — the Library's own refresh
    # on coming back to it — may still be answered after: a 404 for a file
    # that is gone is the right answer from then on.)
    expected.append(to)
    details_do(page, "#lib-d-del")
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
    page.click("#st-src [data-v=gameart]")
    page.select_option("#find-platform", "SNES")
    page.fill("#st-q", "super metroid")
    page.click("#st-go")
    page.wait_for_selector("#find-grid .find-hit", timeout=20000)
    rows = page.eval_on_selector_all("#find-grid .find-hit", "els => els.map(e => e.title)")
    check(any(r.startswith("Super Metroid") for r in rows), f"device studio: 'super metroid' -> {rows[:3]}")
    try:
        page.wait_for_function("[...document.querySelectorAll('#find-grid .find-hit img')].some(i => i.complete && i.naturalWidth)", timeout=20000)
        check(True, f"device studio: pictures load ({page.locator('#find-grid .find-hit').count()} tiles)")
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
    selected GIFs (their copies forgotten), two damaged GIFs written into
    the overlay (one cut short, one with a broken header) and two of the
    card's GIFs with more than 256 colours: `many[0]` with its copy
    forgotten, `many[1]` keeping the card's (lossy, quantised) copy, with a
    .failed line in earlier firmware's words. The first GET of one good GIF
    is aborted (Playwright route): it is retried after 2 s and made. The
    run ends in one summary line; the damaged GIFs are recorded in
    /cache/.failed (POST /api/cache/failed); the two past 256 colours get no
    copy (the lossy one goes), are recorded as playing as they are and count
    as done; a second run fetches none of them. cachetest is back in play,
    "3 of 3", afterwards."""
    good = selected[:2]
    many = ["/gifs/Arcade/ARCADE_SF2_GameOver01.gif", "/gifs/Arcade/ARCADE_SF2_GameOver02.gif"]
    have = {p.lower() for p in get_json(base + "/api/cache/list")["paths"]}
    check(all(cache_path(p).lower() in have for p in many), "damage: the card has lossy copies of the GIFs past 256 colours")
    post_json(base + "/api/cache/forget", {"path": many[0]})
    post_json(base + "/api/cache/failed", {"path": many[1], "reason": "more than 256 colours"})
    src = CARD / good[0].lstrip("/")
    data = src.read_bytes()
    damaged = ["/gifs/Consoles/SNES_Smoke_Damaged_01.gif", "/gifs/Consoles/SNES_Smoke_Damaged_02.gif"]
    for p, b in zip(damaged, [data[: len(data) * 2 // 5], b"GIF00x" + data[6:]]):
        (overlay / p.lstrip("/")).parent.mkdir(parents=True, exist_ok=True)
        (overlay / p.lstrip("/")).write_bytes(b)
    for p in good:
        post_json(base + "/api/cache/forget", {"path": p})
    put_text(base + "/api/sets/damagetest", "".join(p + "\n" for p in good + damaged + many))
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
    watch = lambda r: card(r.url) in damaged + many and hits.append(card(r.url))
    page.route(match, once)
    page.on("request", watch)
    # Chromium's console line for the aborted GET carries its URL.
    expected.append(urllib.parse.quote(base_name(flaky)))
    goto_view(page, "sets")
    goto_view(page, "settings/device")
    sync_app(page)
    wait_cache_line(page, "1 of 6")
    page.click("#pc-go")
    page.wait_for_function("(t => /^\\d+ files? prepared|^Every/.test(t))(document.querySelector('#pc-progress').textContent)", timeout=120000)
    line = page.inner_text("#pc-progress")
    check(line == "4 files prepared · 2 damaged files skipped (they can't be played either)",
          f"damage: summary line '{line}' (first GET of {base_name(flaky)} aborted: {len(aborted)})")
    check(len(aborted) == 1 and page.locator("#pc-net").count() == 0 and page.inner_text("#pc-errors") == "",
          "damage: the aborted download was retried, nothing listed as failed")
    page.click("#pc-damaged summary")
    listed = page.inner_text("#pc-damaged")
    check(all(p in listed for p in damaged) and not any(p in listed for p in many)
          and page.inner_text("#pc-summary").strip().endswith("malformed GIF header"),
          f"damage: the disclosure lists both damaged GIFs, nothing else ({listed!r})")
    check(page.locator("#toasts .toast.bad").count() == 0, "damage: no error toasts")
    shot(page, "settings-damaged", "laptop")
    rows = get_json(base + "/api/cache/failed")
    asis = lambda r: re.match(r"more than 256 colou?rs", r["reason"])
    check(sorted(r["path"] for r in rows if not asis(r)) == damaged and all(r["reason"] for r in rows),
          f"damage: recorded in /cache/.failed ({rows})")
    check(sorted(r["path"] for r in rows if asis(r)) == many
          and any(r["path"] == many[0] and r["reason"] == "more than 256 colours: plays as it is" for r in rows),
          f"damage: the GIFs past 256 colours recorded as playing as they are ({rows})")
    have = {p.lower() for p in get_json(base + "/api/cache/list")["paths"]}
    check(not any(cache_path(p).lower() in have for p in many),
          "damage: no copy of the GIFs past 256 colours (the lossy one removed)")
    line = wait_cache_line(page, "4 of 6")
    check(True, f"damage: the GIFs past 256 colours count as done, '{line}'")
    compare_with_pack(overlay, {cache_path(p): CARD / p.lstrip("/") for p in good})
    n = len(hits)
    page.click("#pc-go")
    page.wait_for_function("(t => /^\\d+ files? prepared|^Every/.test(t))(document.querySelector('#pc-progress').textContent)", timeout=60000)
    line = page.inner_text("#pc-progress")
    check(line == "Every other file is already prepared · 2 damaged files skipped (they can't be played either)"
          and len(hits) == n, f"damage: second run '{line}', damaged GIFs fetched again: {len(hits) - n}")
    page.unroute(match)
    page.remove_listener("request", watch)
    expected.remove(urllib.parse.quote(base_name(flaky)))
    # A GIF that played as it is, replaced by one that fits 256 colours (the
    # bytes of a good GIF, PUT as Add GIFs and Replace do, then its copy made
    # by prepareBytes): the device drops its .failed line, and a run keeps
    # the exact copy. With the stale line back, the run keeps it too.
    rows_of = lambda: {r["path"]: r["reason"] for r in get_json(base + "/api/cache/failed")}
    copies = lambda: {p.lower() for p in get_json(base + "/api/cache/list")["paths"]}
    req = urllib.request.Request(base + "/api/files" + urllib.parse.quote(many[0]), data=data, method="PUT")
    urllib.request.urlopen(req, timeout=30).read()
    check(many[0] not in rows_of(), "replace: a GIF written anew drops its .failed line")
    page.evaluate("async (p) => { const m = await import('./js/prepare.js'); const { app } = await import('./js/state.js');"
                  " await m.prepareBytes(p, await app.src.readBytes(p)); }", many[0])
    check(cache_path(many[0]).lower() in copies(), "replace: the replacement's exact copy is made")
    for stale in (False, True):
        if stale:
            post_json(base + "/api/cache/failed", {"path": many[0], "reason": "more than 256 colours: plays as it is"})
        page.click("#pc-go")
        page.wait_for_function("(t => /^\\d+ files? prepared|^Every/.test(t))(document.querySelector('#pc-progress').textContent)", timeout=60000)
        check(cache_path(many[0]).lower() in copies(),
              f"replace: a run keeps the replacement's exact copy{' (stale line back)' if stale else ''} ('{page.inner_text('#pc-progress')}')")
    post_json(base + "/api/cache/forget", {"path": many[0]})
    (overlay / many[0].lstrip("/")).unlink(missing_ok=True)
    put_json(base + "/api/config", {"set": "cachetest"})
    req = urllib.request.Request(base + "/api/files/sets/damagetest.txt", method="DELETE")
    urllib.request.urlopen(req, timeout=30).read()
    for p in damaged:
        (overlay / p.lstrip("/")).unlink()
    sync_app(page)
    goto_view(page, "sets")
    goto_view(page, "settings/device")
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
# A 1 px-a-frame credits scroll at 50 ms: it moves, but no whole-pixel
# position lies between its frames, so there is nothing to add.
SMOOTH_STEADY = "/gifs/Other/OTHER_SCROLL_The_Oluk_Squid_Game.gif"
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
    page.wait_for_function(f"(document.querySelector('#lib-cap').dataset.shown || '').endsWith({json.dumps(name)})", timeout=30000)
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
    # A 1 px scroll: no switch, but it says why.
    state = preview_in_library(page, SMOOTH_STEADY)
    text = page.inner_text("#lib-smooth-text") if page.locator("#lib-smooth").is_visible() else ""
    check(state == "none" and page.get_attribute("#lib-smooth", "data-hint") == "steady"
          and page.locator("#lib-smooth-on").count() == 0
          and text == "This already moves one pixel at a time, as smooth as the panel can show",
          f"smooth: a 1 px scroll ({SMOOTH_STEADY}) says it is already smooth: '{text}' ({state})")
    # A candidate: the sentence with the CLI's numbers.
    state = preview_in_library(page, SMOOTH_ON)
    text = page.inner_text("#lib-smooth-text") if state in ("on", "off") else ""
    want = f"{int(pct)}% of this animation moves in steps of 2 px or more. Smooth motion adds {added:,} frames"
    check(state == "off" and page.locator("#lib-smooth").is_visible() and text == want,
          f"smooth: shown for {SMOOTH_ON}: '{text}' (analysis {page.get_attribute('#lib-smooth', 'data-analyse-ms')} ms)")
    no_leaks(page, "library (smooth)")
    shot(page, "library-smooth", "laptop")
    # Preview: the smooth version plays in the Library preview, then back.
    page.click("#lib-smooth-preview")
    page.wait_for_function("(document.querySelector('#lib-cap').dataset.shown || '').endsWith(' · smooth')", timeout=60000)
    wait_lit(page, "#lib-panel", "smooth preview")
    check(page.inner_text("#lib-smooth-preview") == "Show original", "smooth: Preview plays the smooth version")
    page.click("#lib-smooth-preview")
    page.wait_for_function(f"(document.querySelector('#lib-cap').dataset.shown || '').endsWith({json.dumps(SMOOTH_ON.rsplit('/', 1)[1])})", timeout=30000)
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
    goto_view(page, "settings/device")
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
        goto_view(page, "settings/time")
        page.wait_for_selector("#fo", timeout=10000)
        for sel in ("#fo-on", "#fo-host", "#fo-code", "#fo-attract", "#fo-test"):
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

        # Saved as changed: follow on, host, code 4321 (when the field is
        # left), attract = the current playlist.
        page.check("#fo-on")
        page.fill("#fo-host", host_val)
        page.fill("#fo-code", "4321")
        page.dispatch_event("#fo-code", "change")
        page.select_option("#fo-attract", "*")
        cfg = wait_config(base, lambda c: c.get("follow") is True and c.get("follow_host") == host_val)
        raw_cfg = json.dumps({k: v for k, v in cfg.items() if k != "follow_host"})  # (the mock's port can hold "4321")
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
        goto_view(page, "settings/time")
        page.wait_for_selector("#fo-on", timeout=10000)
        if page.is_checked("#fo-on"):
            page.uncheck("#fo-on")
        page.fill("#fo-host", "")
        cfg = wait_config(base, lambda c: c.get("follow") is False and c.get("follow_host") == "")
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
    font_checks(page, base, "phone")
    for view in ["library", "sets", "settings"]:
        goto_view(page, view)
        page.wait_for_timeout(1500)
        shot(page, view, "phone")
    overflow = page.evaluate("document.documentElement.scrollWidth - document.documentElement.clientWidth")
    check(overflow <= 0, f"phone: no horizontal scroll ({overflow}px)")
    # And at 360 px, the narrowest phones in use.
    page.set_viewport_size({"width": 360, "height": 780})
    for view in ["now", "library", "sets", "settings", "create"]:
        goto_view(page, view)
        page.wait_for_timeout(500)
        over = page.evaluate("document.documentElement.scrollWidth - document.documentElement.clientWidth")
        check(over <= 0, f"phone 360: {view} has no horizontal scroll ({over}px)")
    for view in ["settings/look", "settings/time", "settings/device", "sets/edit"]:
        goto_view(page, view)
        page.wait_for_timeout(800)
        over = page.evaluate("document.documentElement.scrollWidth - document.documentElement.clientWidth")
        check(over <= 0, f"phone 360: {view} has no horizontal scroll ({over}px)")
    page.set_viewport_size({"width": 390, "height": 844})
    # Playlists on a phone: the editor replaces the list, with a ‹ back.
    goto_view(page, "sets")
    page.wait_for_selector("#sets-list [data-path='/filelist.hall']", timeout=20000)
    check(page.is_hidden("#set-editor") and page.is_hidden(".sehint"), "phone playlists: the list alone, no empty editor")
    page.click("#sets-list [data-path='/filelist.hall'] button.pl")
    page.wait_for_function("location.hash === '#sets/edit'", timeout=10000)
    page.wait_for_selector("#set-entries .vrow[data-path]", timeout=20000)
    label = page.inner_text("#set-name-row > span")
    check(page.is_hidden("#sets-list") and page.inner_text("#set-title") == "Halloween" and label == "Save a copy as"
          and page.input_value("#set-name") == "Halloween copy" and page.inner_text("#set-save") == "Save copy",
          f"phone playlists: Halloween opens as a screen of its own, named, saved as a copy ({label!r})")
    foot = page.eval_on_selector(".edfoot", "e => [getComputedStyle(e).position, Math.round(e.getBoundingClientRect().bottom)]")
    nav = page.eval_on_selector("#nav", "e => Math.round(e.getBoundingClientRect().top)")
    check(foot[0] == "sticky" and foot[1] <= nav, f"phone playlists: Save stays in reach above the nav ({foot}, nav at {nav})")
    shot(page, "sets-editor", "phone")
    page.fill("#set-name", "Mine")
    page.click("#set-build")
    page.wait_for_selector("#builder.sheet", timeout=5000)
    shot(page, "sets-builder", "phone")
    page.wait_for_function("/^Add [1-9]/.test(document.querySelector('#b-append').textContent)", timeout=30000)
    page.click("#b-append")
    page.wait_for_selector("#builder", state="detached", timeout=5000)
    check(page.input_value("#set-name") == "Mine", f"phone playlists: a typed copy name survives Add by filter ({page.input_value('#set-name')!r})")
    page.click("#set-back")
    page.wait_for_function("location.hash === '#sets'", timeout=5000)
    check(page.is_visible("#sets-list") and page.is_hidden("#set-editor"), "phone playlists: ‹ Playlists goes back to the list")
    # The Library's folders on a phone: a tap opens one, another tap closes it.
    goto_view(page, "library")
    pick_cat(page, "Consoles")
    other = page.evaluate(f"(f => f[f.indexOf('Consoles') - 1])({TREE}.folders.map(f => f.cat))")  # the folder just above
    page.click(f"#lib-list .vdir[data-cat={json.dumps(other)}]")
    page.wait_for_function(f"{TREE}.open.has({json.dumps(other)}) && {TREE}.rows.some(r => r.path?.startsWith('/gifs/' + {json.dumps(other)} + '/'))", timeout=20000)
    check(page.locator("#lib-csel, #lib-cats").count() == 0, f"phone library: a tap on {other!r} opens it beside Consoles, no select or tabs")
    pick_cat(page, "Consoles")
    lh = page.eval_on_selector("#lib-list", "e => Math.round(e.getBoundingClientRect().bottom)")
    nav = page.eval_on_selector("#nav", "e => Math.round(e.getBoundingClientRect().top)")
    check(nav - 120 <= lh <= nav, f"phone library: the list reaches down to the nav ({lh} vs nav {nav})")
    # The Library's Details on a phone: a sheet over the list.
    goto_view(page, "library")
    w = page.eval_on_selector("#lib-search", "e => e.getBoundingClientRect().width")
    check(w >= 150, f"phone library: the search box has room ({w:.0f} px)")
    more = page.eval_on_selector("#lib-r2", "e => e.scrollWidth > e.clientWidth + 2 ? e.classList.contains('more') : null")
    page.eval_on_selector("#lib-r2", "e => { e.scrollLeft = e.scrollWidth; }")
    page.wait_for_timeout(150)
    end = page.eval_on_selector("#lib-r2", "e => [e.classList.contains('more'), e.classList.contains('less')]")
    page.eval_on_selector("#lib-r2", "e => { e.scrollLeft = 0; }")
    check(more is not False and end in ([False, True], [False, False]), f"phone library: the chip row fades the side with more off it (start {more}, end {end})")
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
    page.route("https://blob.gifcities.org/**", lambda r: r.fulfill(status=200, body=gif, headers=gc_blob_headers(r.request)))
    goto_view(page, "create")
    page.wait_for_selector("#view-create.on #studio .st-grid", timeout=20000)
    page.click("#st-src [data-v=gifcities]")
    page.fill("#st-q", "mario")
    page.click("#st-go")
    page.wait_for_function("document.querySelector('#find-count').textContent === '3 of 3'", timeout=10000)
    wide = page.evaluate("document.documentElement.scrollWidth - document.documentElement.clientWidth")
    check(wide <= 0, f"phone create art: GifCities results fit ({wide}px over)")
    shot(page, "create-find", "phone")
    page.click("#st-src [data-v=gameart]")
    goto_view(page, "library")
    # The Studio on a phone, from the nav: one column, nothing wider than the screen.
    hits: list[str] = []
    stub_art(page, hits)
    open_studio(page, "#nav-art")
    # Nothing in yet: only step 1, Find.
    shown = page.eval_on_selector_all("#studio .card", "els => els.filter(e => e.offsetParent).map(e => e.id)")
    check(shown == ["st-import"] and not page.is_visible("#st-change"), f"phone studio: empty, only Find shows ({shown})")
    page.evaluate("scrollTo(0, 0)")
    shot(page, "studio-empty", "phone")
    page.set_input_files("#st-file", str(OUT / "studio-fixture.png"))
    page.wait_for_function("document.querySelector('#studio').dataset.frames", timeout=20000)
    boxes = studio_boxes(page)
    check(all(b[0] == boxes[0][0] for b in boxes) and [b[1] for b in boxes] == sorted(b[1] for b in boxes),
          "phone studio: one column, 1-6")
    page.click("#st-change")
    page.click("#st-src [data-v=gameart]")
    page.select_option("#find-platform", "SNES")
    page.fill("#st-q", "studio")
    page.click("#st-go")
    page.wait_for_selector("#find-grid .find-hit")
    wide = page.evaluate("document.documentElement.scrollWidth - document.documentElement.clientWidth")
    check(wide <= 0, f"phone studio: no horizontal scroll ({wide}px)")
    no_leaks(page, "phone studio")
    studio_shots(page, "phone")
    # An animation on a phone: the timeline and the Trim / Motion groups fit.
    page.evaluate("delete document.querySelector('#studio').dataset.tImport")
    page.set_input_files("#st-file", str(OUT / "anim-fixture.gif"))
    page.wait_for_function("document.querySelector('#studio').dataset.tImport", timeout=30000)
    page.evaluate("document.querySelector('#st-preview').scrollIntoView()")
    page.wait_for_timeout(300)
    wide = page.evaluate("document.documentElement.scrollWidth - document.documentElement.clientWidth")
    check(wide <= 0 and page.is_visible("#st-tl"), f"phone studio: an animation's timeline, no horizontal scroll ({wide}px)")
    shot(page, "studio-anim", "phone")
    page.context.close()


def hostname_checks(page: Page, base: str) -> None:
    """A new hostname is live at once: the reply asks for no reboot (a WiFi
    change still does), and Save network says where the display is now —
    from an IP without a link, from a .local address (Chromium resolves
    smoke-*.local to the mock: LOCAL_RULE) with one to the new address,
    which loads; on a clash (status `mdns` <name>-2, faked) the link goes
    there."""
    before = get_json(base + "/api/config")["hostname"]
    r = put_json(base + "/api/config", {"brightness": get_json(base + "/api/config")["brightness"]})
    check(r.get("reboot_required") is False, f"config: no reboot for a brightness write ({r.get('reboot_required')})")
    r = put_json(base + "/api/config", {"hostname": before + "-x"})
    check(r.get("reboot_required") is False, f"config: a hostname change is live, no reboot_required ({r.get('reboot_required')})")
    st = get_json(base + "/api/status")
    check(st["hostname"] == st["mdns"] == before + "-x", f"status: hostname and mdns are the new name at once ({st['hostname']}, {st['mdns']})")
    ssid = get_json(base + "/api/config")["wifi_ssid"]
    r = put_json(base + "/api/config", {"wifi_ssid": ssid + "-x"})
    check(r.get("reboot_required") is True, f"config: a WiFi change still replies reboot_required true ({r.get('reboot_required')})")
    put_json(base + "/api/config", {"hostname": before, "wifi_ssid": ssid})
    page.evaluate("async () => { const { app } = await import('./js/state.js'); await app.refreshConfig(); }")
    page.wait_for_function(f"document.querySelector('#st-hostname').value === {json.dumps(before)}", timeout=5000)
    check("WiFi changes need a reboot" in page.inner_text("#st-net") and "name changes" not in page.inner_text("#st-net"),
          "settings: the Network note says only WiFi changes need a reboot")
    page.fill("#st-hostname", before + "-smoke")
    page.click("#st-save-net")
    page.wait_for_selector("#st-moved", state="visible", timeout=5000)
    applying = page.inner_text("#st-moved")
    page.wait_for_function("document.querySelector('#st-moved').textContent.includes('.local')", timeout=15000)
    moved = page.inner_text("#st-moved")
    check("Applying" in applying, f"settings: Save network with a new name says it is being applied first ({applying!r})")
    check(f"{before}-smoke.local" in moved and page.query_selector("#st-goto") is None and page.is_hidden("#st-reboot")
          and get_json(base + "/api/config")["hostname"] == before + "-smoke",
          f"settings: from an IP, Save network with a new name says where it is now, no link, no reboot prompt ({moved!r})")

    # No mDNS (status `mdns` "", as on the setup network): saved, no address.
    page.route(STATUS_RE, lambda route: route.fulfill(json={**get_json(base + "/api/status"), "mdns": ""}))
    page.fill("#st-hostname", before + "-nomdns")
    page.click("#st-save-net")
    page.wait_for_function("document.querySelector('#st-moved').textContent.trim() === 'Saved.'", timeout=15000)
    moved = page.inner_text("#st-moved")
    check(".local" not in moved, f"settings: with no mDNS a new name is saved with no address claimed ({moved!r})")
    page.unroute(STATUS_RE)
    # A later Save network that does not rename hides the notice.
    page.fill("#st-ssid", page.input_value("#st-ssid") + "-x")
    page.click("#st-save-net")
    page.wait_for_selector("#st-reboot", state="visible", timeout=5000)
    check(page.is_hidden("#st-moved"), "settings: a later WiFi-only save hides the name notice and shows the reboot prompt")
    put_json(base + "/api/config", {"wifi_ssid": ssid})
    page.evaluate("document.querySelector('#st-reboot').hidden = true")

    # A name another display has (the mock's --taken): said under the field
    # when it is left, and Save network sends nothing; if the check could
    # not tell (taken null, faked), the device's 409 is explained instead.
    cfg0 = get_json(base + "/api/config")
    puts: list[str] = []
    on_put = lambda r: r.method == "PUT" and "/api/config" in r.url and puts.append(r.url)
    page.on("request", on_put)
    page.fill("#st-hostname", TAKEN_NAME)
    page.dispatch_event("#st-hostname", "change")
    page.wait_for_selector("#st-name-taken:not([hidden])", timeout=10000)
    t = page.inner_text("#st-name-taken")
    check(t == f"Another display on this network ({TAKEN_BY}) is already called “{TAKEN_NAME}”. Pick a different name.",
          f"settings: a taken name is said under the field when it is left ({t!r})")
    page.fill("#st-ssid", cfg0["wifi_ssid"] + "-x")
    page.click("#st-save-net")
    page.wait_for_timeout(1500)
    check(not puts and get_json(base + "/api/config") == cfg0 and page.is_visible("#st-name-taken"),
          f"settings: Save network with a taken name sends nothing ({puts})")
    page.route("**/api/name-check*", lambda r: r.fulfill(json={"name": TAKEN_NAME, "valid": True, "taken": None, "by": None}))
    expected.append("/api/config")  # the 409 is the point
    page.fill("#st-hostname", TAKEN_NAME)
    page.click("#st-save-net")
    page.wait_for_function("document.querySelector('#st-name-taken').textContent.includes('nothing was saved')", timeout=10000)
    page.wait_for_timeout(300)
    expected.remove("/api/config")
    t = page.inner_text("#st-name-taken")
    after = get_json(base + "/api/config")
    check(t == f"“{TAKEN_NAME}” is taken by another display ({TAKEN_BY}); nothing was saved." and len(puts) == 1
          and after["hostname"] == cfg0["hostname"] and after["wifi_ssid"] == cfg0["wifi_ssid"] and page.is_hidden("#st-reboot"),
          f"settings: the device's 409 is explained and neither the name nor the WiFi was saved ({t!r})")
    page.unroute("**/api/name-check*")
    page.remove_listener("request", on_put)
    page.fill("#st-hostname", cfg0["hostname"])
    page.fill("#st-ssid", cfg0["wifi_ssid"])
    page.dispatch_event("#st-hostname", "input")
    check(page.is_hidden("#st-name-taken"), "settings: typing again clears the message")

    port = urllib.parse.urlsplit(base).port
    put_json(base + "/api/config", {"hostname": "smoke-a"})
    lp = page.context.new_page()
    errs: list[str] = []
    lp.on("pageerror", lambda e: errs.append(str(e)))
    lp.goto(f"http://smoke-a.local:{port}/#settings")
    goto_view(lp, "settings/device")
    lp.wait_for_function("document.querySelector('#st-hostname').value === 'smoke-a'", timeout=10000)
    lp.fill("#st-hostname", "smoke-b")
    lp.click("#st-save-net")
    lp.wait_for_selector("#st-goto", timeout=15000)
    want = lp.evaluate("location.href").replace("smoke-a.local", "smoke-b.local")
    href = lp.get_attribute("#st-goto", "href")
    check(href == want and "smoke-b.local" in lp.inner_text("#st-moved"),
          f"settings: from a .local address, a link to the new one ({href})")
    lp.click("#st-goto")
    lp.wait_for_url(f"http://smoke-b.local:{port}/**", timeout=10000)
    goto_view(lp, "settings/device")
    lp.wait_for_function("document.querySelector('#st-hostname').value === 'smoke-b'", timeout=10000)
    check(True, "settings: the link loads the app at the new address")

    def clash(route):
        # (Asked of the mock by its IP: Playwright's own fetch does not use
        # Chromium's resolver rule.)
        j = get_json(base + "/api/status")
        route.fulfill(json={**j, "mdns": j["hostname"] + "-2"})
    lp.route(STATUS_RE, clash)
    lp.fill("#st-hostname", "smoke-c")
    lp.click("#st-save-net")
    lp.wait_for_function("document.querySelector('#st-goto')?.href.includes('smoke-c')", timeout=15000)
    href = lp.get_attribute("#st-goto", "href")
    check(urllib.parse.urlsplit(href).hostname == "smoke-c-2.local" and "smoke-c-2.local" in lp.inner_text("#st-moved"),
          f"settings: on a clash the link goes to the name mDNS took ({href})")
    lp.unroute(STATUS_RE)
    check(not errs, f"settings: no page errors at the .local addresses ({errs})")
    lp.close()
    put_json(base + "/api/config", {"hostname": before})
    page.evaluate("async () => { const { app } = await import('./js/state.js'); await app.refreshConfig(); }")
    page.evaluate("document.querySelector('#st-moved').hidden = true")


# Chromium resolves these names to the mock, so a page can move to
# <new>.local as on a real network (hostname_checks). Only smoke-*.local:
# a DEVICE=http://<name>.local run still resolves for real.
LOCAL_RULE = "MAP smoke-*.local 127.0.0.1"


STATUS_RE = re.compile(r"/api/status(\?|$)")


def refresh_app(page: Page) -> None:
    page.evaluate("async () => { const { app } = await import('./js/state.js'); await app.refreshStatus(); await app.refreshConfig(); }")


def names_checks(page: Page, base: str) -> None:
    """The display's name: PUT /api/config lowercases and validates it
    (opendmd_core::hostname::valid); status `mdns` is the name mDNS answers
    to, and when it differs (another display had the name: faked here)
    Settings → About and the setup page say where this one is. The setup
    page's Display name is prefilled, checked, and saved with the WiFi."""
    before = get_json(base + "/api/config")["hostname"]
    for bad in ("Bad Name", "-rig", "rig-", "a" * 33, "rig.two"):
        check(put_status(base, {"hostname": bad}) == 400, f"config: hostname {bad!r} refused (400)")
    put_json(base + "/api/config", {"hostname": " Rig-Two "})
    check(get_json(base + "/api/config")["hostname"] == "rig-two", "config: hostname trimmed and lowercased (rig-two)")
    put_json(base + "/api/config", {"hostname": before})
    name_taken_checks(base)
    st = get_json(base + "/api/status")
    check(st.get("mdns") == st["hostname"], f"status: mdns is the hostname with no clash ({st.get('mdns')})")
    label = page.inner_text("#view-settings label:has(#st-hostname)")
    check("Display name" in label and ".local" in page.inner_text("#st-net"), f"settings: the field is called Display name ({label!r})")

    def clash(route):
        r = route.fetch()
        j = r.json()
        route.fulfill(response=r, json={**j, "mdns": j["hostname"] + "-2"})
    page.route(STATUS_RE, clash)
    refresh_app(page)
    page.wait_for_selector("#st-mdns", timeout=5000)
    about = page.inner_text("#st-about")
    check(f"{before}-2.local" in about and f"“{before}”" in page.inner_text("#st-mdns"),
          f"settings: About shows the real address and why ({about!r})")
    shot(page, "settings-mdns-clash", "laptop")
    page.unroute(STATUS_RE)
    refresh_app(page)
    page.wait_for_selector("#st-mdns", state="detached", timeout=5000)

    # The setup page (/setup): its own tab, the API answered for real
    # except the save and the reboot, which are caught.
    sp = page.context.new_page()
    sp.route(STATUS_RE, clash)
    sent: list[dict] = []

    def save(route):
        if route.request.method == "PUT":
            sent.append(json.loads(route.request.post_data or "{}"))
            route.fulfill(status=200, json={"ok": True, "reboot_required": True})
        else:
            route.continue_()
    sp.route("**/api/config", save)
    sp.route("**/api/reboot", lambda r: r.fulfill(status=200, json={"ok": True, "rebooting": True}))
    sp.goto(base + "/setup")
    sp.wait_for_function(f"document.querySelector('#host').value === {json.dumps(before)}", timeout=10000)
    check(f"http://{before}.local/" in sp.inner_text("#hosturl"), "setup: Display name prefilled with the hostname, its address shown")
    sp.wait_for_selector("#hostnote", state="visible", timeout=10000)
    check(f"{before}-2.local" in sp.inner_text("#hostnote"), "setup: a name clash is explained under the field")
    sp.fill("#ssid", "SmokeNet")
    sp.fill("#host", "Bad Name")
    sp.click("#savewifi")
    check("display name" in sp.inner_text("#wifimsg") and not sent, "setup: a bad name is refused before anything is sent")
    sp.fill("#host", "Rig-Two")
    check(sp.inner_text("#hosturl") == "http://rig-two.local/", "setup: the address follows the name as typed")
    sp.click("#savewifi")
    sp.wait_for_function("document.querySelector('#wifimsg').textContent.startsWith('Saved')", timeout=5000)
    check(sent == [{"wifi_ssid": "SmokeNet", "hostname": "rig-two"}] and "http://rig-two.local/" in sp.inner_text("#wifimsg"),
          f"setup: Connect saves the name with the WiFi and says where to go next ({sent})")
    shot(sp, "setup-name", "laptop")
    # A name another device answers to: the mock's real 409, nothing saved.
    sp.unroute("**/api/config")
    cfg = get_json(base + "/api/config")
    sp.fill("#host", TAKEN_NAME)
    sp.click("#savewifi")
    sp.wait_for_function("document.querySelector('#wifimsg').textContent.startsWith('Another display')", timeout=10000)
    after = get_json(base + "/api/config")
    check(f"“{TAKEN_NAME}”" in sp.inner_text("#wifimsg") and after["wifi_ssid"] == cfg["wifi_ssid"] and after["hostname"] == before,
          f"setup: a taken name (409) says so, nothing saved ({sp.inner_text('#wifimsg')!r})")
    sp.close()


# A name the mock says another device answers to (mock-device.py --taken's
# default).
TAKEN_NAME, TAKEN_BY = "other-display", "192.168.1.42"


def name_taken_checks(base: str) -> None:
    """GET /api/name-check answers {name, valid, taken, by} (the name
    trimmed and lowercased; taken null for an invalid name; the current
    name never taken), and PUT /api/config refuses a hostname another device
    answers to: 409 "name taken: …" with name and by, nothing saved, the
    rest of the patch included. API-level only (no Settings selectors)."""
    def nc(name: str) -> dict:
        return get_json(base + "/api/name-check?name=" + urllib.parse.quote(name))
    cfg = get_json(base + "/api/config")
    r = nc(" Other-Display ")
    check(r == {"name": TAKEN_NAME, "valid": True, "taken": True, "by": TAKEN_BY},
          f"name-check: a name another device answers to is taken, by its address ({r})")
    r = nc("smoke-free")
    check(r == {"name": "smoke-free", "valid": True, "taken": False, "by": None}, f"name-check: a free name ({r})")
    r = nc("bad.name")
    check(r == {"name": "bad.name", "valid": False, "taken": None, "by": None}, f"name-check: an invalid name, not asked ({r})")
    r = nc(cfg["hostname"])
    check(r["valid"] and r["taken"] is False, f"name-check: the display's own name is not taken ({r})")
    try:
        get_json(base + "/api/name-check")
        code = 200
    except urllib.error.HTTPError as e:
        code = e.code
    check(code == 400, f"name-check: no name= is a 400 ({code})")
    bright = 33 if cfg["brightness"] != 33 else 34
    try:
        put_json(base + "/api/config", {"hostname": TAKEN_NAME, "brightness": bright})
        code, body = 200, {}
    except urllib.error.HTTPError as e:
        code, body = e.code, json.load(e)
    after = get_json(base + "/api/config")
    check(code == 409 and body.get("error", "").startswith("name taken") and body.get("by") == TAKEN_BY
          and body.get("name") == TAKEN_NAME,
          f"config: a taken hostname is a 409 NAME_TAKEN with name and by ({code} {body})")
    check(after["hostname"] == cfg["hostname"] and after["brightness"] == cfg["brightness"],
          f"config: nothing of a refused patch saved ({after['hostname']}, {after['brightness']})")
    # Re-saving the current name is never refused (not asked).
    check(put_status(base, {"hostname": cfg["hostname"]}) == 200, "config: re-saving the current name is fine")


def platform_checks(page: Page, base: str) -> None:
    """Settings → Playback's platform chips (the playlist's platforms with
    their counts, "All" when none is picked) save as tapped; a filter that
    would leave the playlist empty is refused (400, nothing saved), alone
    or with a set; and Now explains an empty playlist, with Clear filter."""
    cfg = get_json(base + "/api/config")
    check(cfg["platforms"] == "" and cfg["set"] == "", f"platforms: start from All files, no filter ({cfg['platforms']!r})")
    goto_view(page, "settings/play")
    total = get_json(base + "/api/status")["entries"]
    page.wait_for_function("/^All \\(\\d+\\)$/.test(document.querySelector('#st-plat-val').textContent)", timeout=30000)
    check(page.locator("#st-plats").count() == 0 and page.locator("#st-platforms").count() == 0,
          f"settings: platforms are one row, '{page.inner_text('#st-plat-val')}', no chips until Choose…")
    page.click("#st-plat-btn")
    page.wait_for_selector("#st-plat-sheet #st-plats [data-platform='SNES']", timeout=30000)
    check(page.is_disabled("#st-plat-only"), "settings: the platform sheet opens with nothing picked, Only these disabled")
    check(page.evaluate("document.activeElement.id") == "st-plat-q", "settings: the focus moves into the sheet (its search box)")
    page.keyboard.press("Escape")
    check(page.locator("#st-plat-sheet").count() == 0 and page.evaluate("document.activeElement.id") == "st-plat-btn",
          "settings: Escape closes the sheet and gives the focus back to Choose…")
    page.click("#st-plat-btn")
    page.wait_for_selector("#st-plat-sheet", timeout=5000)
    goto_view(page, "now")
    check(page.locator("#st-plat-sheet").count() == 0, "settings: a sheet does not outlive its view (closed on the way to Now)")
    goto_view(page, "settings/play")
    page.click("#st-plat-btn")
    page.wait_for_selector("#st-plat-sheet #st-plats [data-platform='SNES']", timeout=30000)
    counts = page.eval_on_selector_all("#st-plats .chip[data-platform]", "els => els.map(e => +e.querySelector('small').textContent.replace(/,/g, ''))")
    more = page.locator("#st-plat-more").count()
    check(counts == sorted(counts, reverse=True) and (not more or min(counts) >= 5),
          f"settings: chips most first, those under 5 behind +N more ({counts}, more {more})")
    if more:
        page.click("#st-plat-more")
        check(page.locator("#st-plat-more").count() == 0, "settings: +N more shows the rest")
    page.fill("#st-plat-q", "sn")
    shown = page.eval_on_selector_all("#st-plats .chip[data-platform]", "els => els.map(e => e.dataset.platform)")
    check("SNES" in shown and "MD" not in shown, f"settings: the sheet's search narrows the chips ({shown})")
    page.click("#st-plats [data-platform='SNES']")
    page.fill("#st-plat-q", "")
    page.click("#st-plats [data-platform='MD']")
    shot(page, "settings-platforms", "laptop")
    check(get_json(base + "/api/config")["platforms"] == "" and page.inner_text("#st-plat-only") == "Only these (2)",
          "settings: nothing saved until Only these")
    page.click("#st-plat-only")
    c = wait_config(base, lambda c: set(c["platforms"].split(",")) == {"SNES", "MD"})
    check(set(c["platforms"].split(",")) == {"SNES", "MD"} and page.locator("#st-plat-sheet").count() == 0,
          f"settings: Only these saves SNES and MD and closes the sheet ({c['platforms']!r})")
    page.wait_for_function("document.querySelector('#st-plat-val').textContent === 'SNES, MD'", timeout=5000)
    check(True, "settings: the row names them")
    entries = get_json(base + "/api/status")["entries"]
    check(0 < entries < total, f"platforms: MD,SNES plays some of the playlist ({entries} of {total})")
    # Refused: the filter alone, and with a set; nothing saved.
    try:
        put_json(base + "/api/config", {"platforms": "Loo"})
        code, msg = 200, ""
    except urllib.error.HTTPError as e:
        code, msg = e.code, json.load(e).get("error", "")
    after = get_json(base + "/api/config")
    check(code == 400 and '"Loo"' in msg and set(after["platforms"].split(",")) == {"SNES", "MD"}
          and get_json(base + "/api/status")["entries"] == entries,
          f"config: platforms Loo (nothing would play) is a 400 naming it, nothing saved ({code} {msg!r})")
    check(put_status(base, {"platforms": "Loo", "set": "filelist.xmas", "brightness": 33}) == 400
          and get_json(base + "/api/config")["set"] == "" and get_json(base + "/api/config")["brightness"] != 33,
          "config: Loo with a set change is refused too, nothing of the patch saved")
    page.click("#st-plat-btn")
    page.wait_for_selector("#st-plats [data-platform='MD'].on", timeout=10000)
    page.click("#st-plat-all")
    c = wait_config(base, lambda c: c["platforms"] == "")
    check(c["platforms"] == "", "settings: All platforms clears the filter")
    # Now, with nothing to play under a filter (faked: the firmware refuses
    # to get there, but a filter saved by older firmware can).
    goto_view(page, "now")

    def empty(route):
        r = route.fetch()
        j = r.json()
        route.fulfill(response=r, json={**j, "entries": 0, "index_state": "unavailable",
                                        "now_playing": {**j["now_playing"], "path": "", "title": "", "ordinal": 0}})
    page.route(STATUS_RE, empty)
    page.evaluate("async () => { const { app } = await import('./js/state.js'); app.config.platforms = 'Loo'; }")
    page.wait_for_function("document.querySelector('#now-title').textContent.includes('platform filter (Loo)')", timeout=5000)
    check(page.is_visible("#now-unfilter"), "now: an empty playlist names the platform filter, with Clear filter")
    shot(page, "now-empty-filter", "laptop")
    page.click("#now-unfilter")
    page.wait_for_function("document.querySelector('#now-title').textContent.includes('playlist is empty')", timeout=5000)
    check(page.is_hidden("#now-unfilter"), "now: Clear filter clears it (the message says the playlist is empty)")
    page.unroute(STATUS_RE)

    # No card: "No card" once a mount attempt has failed, nothing claimed
    # while the card is still being mounted at boot.
    def nocard(sd):
        def f(route):
            r = route.fetch()
            j = r.json()
            route.fulfill(response=r, json={**j, "entries": 0, "index_state": "unavailable", "sd": {**j["sd"], **sd},
                                            "now_playing": {**j["now_playing"], "path": "", "title": "", "ordinal": 0}})
        return f
    page.route(STATUS_RE, nocard({"mounted": False, "readable": False, "attempts": 0, "error": None}))
    page.wait_for_function("document.querySelector('#now-title').textContent === 'Nothing playing'", timeout=5000)
    check(True, "now: no card claimed while the card is still mounting (no attempt yet)")
    page.unroute(STATUS_RE)
    page.route(STATUS_RE, nocard({"mounted": False, "readable": False, "attempts": 2,
                                  "error": "no card, or card init failed: ESP_ERR_TIMEOUT"}))
    page.wait_for_function("document.querySelector('#now-title').textContent === 'No card'", timeout=5000)
    check(page.is_hidden("#now-unfilter"), "now: a failed mount says No card")
    page.unroute(STATUS_RE)
    page.wait_for_function("!document.querySelector('#now-title').textContent.startsWith('Nothing')", timeout=10000)
    check(True, "now: playing again once status has entries")
    goto_view(page, "settings/device")
    page.wait_for_selector("#st-facts dt")


def put_www(base: str, body: bytes) -> tuple[int, dict | None]:
    req = urllib.request.Request(base + "/api/www", data=body, method="PUT",
                                 headers={"Content-Type": "application/octet-stream"})
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return r.status, json.load(r)
    except urllib.error.HTTPError as e:
        return e.code, None


def www_mock_checks(page: Page, base: str, version: str) -> None:
    """GET/PUT /api/www against the mock; the OTA card is gone."""
    w = get_json(base + "/api/www")
    check(sorted(w) == ["bytes", "entries", "error", "ok", "staged", "version"] and w["ok"] and w["version"] == version
          and w["entries"] > 10 and w["staged"] is None, f"web app: GET /api/www {w}")
    st = get_json(base + "/api/status")
    check(st.get("www") == w and st.get("www_version") == version, "web app: /api/status carries www and www_version")
    pack = REPO / "opendmd-web" / "www.pack"
    for body, want in ((pack.read_bytes(), 200), (b"not a pack at all", 400)):
        code, reply = put_www(base, body)
        check(code == want, f"web app: PUT /api/www ({len(body):,} B) -> {code}")
        if want == 200:
            check(reply is not None and reply.get("staged") is True and get_json(base + "/api/www")["staged"] is True,
                  f"web app: PUT /api/www with a card replies staged true, and GET /api/www keeps it ({reply})")
        else:
            check(get_json(base + "/api/www")["staged"] is None,
                  "web app: a PUT refused at its header leaves staged null (it describes the last attempt)")


def post_code(url: str, obj) -> tuple[int, dict | None]:
    """A JSON POST's status and body, errors included."""
    req = urllib.request.Request(url, data=json.dumps(obj).encode(), method="POST",
                                 headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return r.status, json.load(r)
    except urllib.error.HTTPError as e:
        try:
            return e.code, json.load(e)
        except ValueError:
            return e.code, None


def sd_unreadable_pass(browser) -> None:
    """`mock-device.py --sd unreadable`: a card that mounts but cannot be
    read (exFAT). Status says so, the Now page names it, nothing is staged;
    then Now's "Format card…" dialog formats it and the card reads, and a
    second format is refused (409: only an unreadable card is formatted)."""
    proc, base = start_mock(["--root", str(CARD), "--sd", "unreadable"])
    errors: list[str] = []
    try:
        print(f"\n== mock device with an unreadable card at {base}")
        sd = get_json(base + "/api/status")["sd"]
        check(sd["mounted"] is True and sd["readable"] is False and "FAT32" in (sd["error"] or "")
              and "32 GB" not in sd["error"] and sd["formatting"] is False,
              f"unreadable card: status sd {{mounted: true, readable: false, error, formatting: false}} ({sd})")
        page = new_page(browser, "laptop", errors)
        page.goto(base + "/")
        page.wait_for_selector("#now-sd:not([hidden])", timeout=20000)
        check(page.is_visible("#now-sd") and page.is_visible("#now-format"),
              "unreadable card: Now shows the card line and Format card…")
        goto_view(page, "settings/device")
        page.wait_for_function("[...document.querySelectorAll('#st-facts dt')].some(e => e.textContent === 'Card')", timeout=20000)
        card = page.evaluate("() => { const dt = [...document.querySelectorAll('#st-facts dt')].find(e => e.textContent === 'Card');"
                             " const dd = dt.nextElementSibling; return [dd.textContent, dd.classList.contains('bad')]; }")
        check(card == ["Not readable: needs FAT32", True], f"unreadable card: Settings › Device's Card line says so, in red ({card})")
        goto_view(page, "now")
        shot(page, "now-sd-unreadable", "laptop")
        code, reply = put_www(base, (REPO / "opendmd-web" / "www.pack").read_bytes())
        check(code == 200 and reply is not None and reply.get("staged") is False,
              f"unreadable card: PUT /api/www is not staged ({code}, staged {reply and reply.get('staged')})")
        code, reply = post_code(base + "/api/card/format", {})
        check(code == 400, f"card format: no confirm is a 400 ({code} {reply})")
        # The dialog: Cancel changes nothing.
        page.click("#now-format")
        page.wait_for_selector("#fmt-dlg", state="visible", timeout=5000)
        text = page.inner_text("#fmt-dlg")
        check("Everything on it is erased" in text and "exFAT" in text and "FAT32" in text,
              f"card format: the dialog warns and explains ({text!r})")
        check(page.evaluate("() => document.querySelector('#fmt-go').classList.contains('danger')"),
              "card format: the destructive button is red (.danger)")
        shot(page, "card-format-dialog", "laptop")
        page.click("#fmt-close")
        page.wait_for_selector("#fmt-dlg", state="detached", timeout=5000)
        check(get_json(base + "/api/status")["sd"]["readable"] is False, "card format: Cancel leaves the card alone")
        # Format: progress, then the card reads.
        page.click("#now-format")
        page.click("#fmt-go")
        page.wait_for_function("document.querySelector('#fmt-msg')?.textContent.startsWith('Formatting')", timeout=5000)
        check(page.is_visible("#fmt-prog") and page.is_disabled("#fmt-close"), "card format: progress shown, the dialog held open")
        busy = get_json(base + "/api/status")["sd"]
        check(busy["formatting"] is True, f"card format: status sd.formatting while it runs ({busy})")
        code, reply = post_code(base + "/api/card/format", {"confirm": True})
        check(code == 409 and "already" in (reply or {}).get("error", ""), f"card format: a second one while running is a 409 ({code} {reply})")
        page.wait_for_function("document.querySelector('#fmt-msg')?.textContent.startsWith('Done')", timeout=20000)
        shot(page, "card-format-done", "laptop")
        sd = get_json(base + "/api/status")["sd"]
        check(sd["readable"] is True and sd["formatting"] is False and sd["format_error"] is None,
              f"card format: the card reads afterwards ({sd})")
        page.click("#fmt-close")
        page.wait_for_selector("#now-sd", state="hidden", timeout=5000)
        code, reply = post_code(base + "/api/card/format", {"confirm": True})
        check(code == 409 and "can be read" in (reply or {}).get("error", ""),
              f"card format: a readable card is a 409 ({code} {reply})")
        page.context.close()
    finally:
        proc.terminate()
    report_errors("mock (unreadable card)", errors)


def sd_slowed_pass(browser) -> None:
    """`mock-device.py --sd slowed`: Settings › Device's Card speed."""
    proc, base = start_mock(["--root", str(CARD), "--sd", "slowed"])
    errors: list[str] = []
    try:
        print(f"\n== mock device with a slowed card at {base}")
        sd = get_json(base + "/api/status")["sd"]
        check(sd["clock_khz"] == 20000 and sd["fallback"] is True and sd["slowdowns"] == 1 and sd["probe"] == "slowed",
              f"slowed card: status sd 20 MHz, fallback, slowdowns 1, probe slowed ({sd})")
        page = new_page(browser, "laptop", errors)
        page.goto(base + "/")
        goto_view(page, "settings/device")
        page.wait_for_selector("#st-cs:not([hidden])", timeout=20000)
        check(page.is_visible("#st-cs") and page.input_value("#st-card-speed") == "fast",
              "card speed: the group is shown, Fast selected")
        card = lambda: page.evaluate("() => { const dt = [...document.querySelectorAll('#st-facts dt')].find(e => e.textContent === 'Card');"
                                     " return dt && dt.nextElementSibling.textContent; }")
        page.wait_for_function("document.querySelector('#st-card-slow') && !document.querySelector('#st-card-slow').hidden", timeout=20000)
        check("· Safe speed" in (card() or ""), f"card speed: the Card fact says Safe speed ({card()!r})")
        check("runs at Safe" in page.inner_text("#st-card-slow"), "card speed: the slowed-from-Fast line is shown")
        shot(page, "card-speed-slowed", "laptop")
        page.click("#st-card-full")
        page.wait_for_selector("#st-card-slow", state="hidden", timeout=10000)
        page.wait_for_function("[...document.querySelectorAll('#st-facts dt')].some(e => e.textContent === 'Card' && e.nextElementSibling.textContent.includes('Fast speed'))", timeout=10000)
        sd = get_json(base + "/api/status")["sd"]
        check(sd["clock_khz"] == 40000 and sd["fallback"] is False and "Fast speed" in card(),
              f"card speed: Try Fast again clears the slow line, Card reads Fast speed ({card()!r}, {sd})")
        # The select: Safe and Fast (full is API-only).
        check(page.evaluate("[...document.querySelectorAll('#st-card-speed option')].map(o => o.value).join()") == "safe,fast",
              "card speed: the select offers Safe and Fast only")
        def pick(v: str, n: int) -> None:
            page.select_option("#st-card-speed", v)
            page.wait_for_function(f"+document.querySelector('#st-cs h3 .tick').dataset.n >= {n}", timeout=10000)
        pick("safe", 1)
        check(page.text_content("#st-cs h3 .tick") == "Saved ✓", "card speed: Safe shows Saved ✓")
        cfg = get_json(base + "/api/config")
        sd = get_json(base + "/api/status")["sd"]
        check(cfg["card_speed"] == "safe" and sd["clock_khz"] == 20000 and sd["fallback"] is False,
              f"card speed: Safe saved, 20 MHz without fallback ({cfg['card_speed']}, {sd})")
        check("Plenty fast" in page.text_content("#st-cs-cap") and not page.evaluate("document.querySelector('#st-cs-cap').classList.contains('bad')"),
              f"card speed: the Safe caption ({page.text_content('#st-cs-cap')!r})")
        code, reply = post_code(base + "/api/card/full-speed", {})
        check(code == 409 and "card_speed is safe" in (reply or {}).get("error", ""), f"card speed: full-speed with Safe is a 409 ({code} {reply})")
        pick("fast", 2)
        sd = get_json(base + "/api/status")["sd"]
        check(get_json(base + "/api/config")["card_speed"] == "fast" and sd["clock_khz"] == 40000 and sd["fallback"] is False
              and "wrong data" in page.text_content("#st-cs-cap") and page.is_hidden("#st-card-slow"),
              f"card speed: Fast saved, 40 MHz, its caution, no slow line ({sd})")
        def put(v: str):
            req = urllib.request.Request(base + "/api/config", data=json.dumps({"card_speed": v}).encode(), method="PUT",
                                         headers={"Content-Type": "application/json"})
            try:
                return 200, json.load(urllib.request.urlopen(req, timeout=30))
            except urllib.error.HTTPError as e:
                return e.code, json.load(e)
        code, reply = put("full")
        check(code == 200 and get_json(base + "/api/config")["card_speed"] == "full" and get_json(base + "/api/status")["sd"]["clock_khz"] == 40000,
              f"card speed: full (API only) saved, 40 MHz ({code} {reply})")
        code, reply = put("auto")
        check(code == 400 and reply.get("error") == "card_speed must be safe, fast or full", f"card speed: auto (gone) is a 400 ({code} {reply})")
        put("safe")
        page.context.close()
    finally:
        proc.terminate()
    report_errors("mock (slowed card)", errors)


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
    check(page.locator("#now-skip").count() == 1,
          f"device web app: the page from flash renders the Now view ({time.time() - t0:.1f} s)")
    wait_lit(page, "#now-panel", "device web app", 40)
    goto_view(page, "settings/device")
    page.wait_for_selector("#st-facts dt", timeout=20000)
    check(w.get("version", "?") in page.inner_text("#st-about"), "device web app: Settings shows the pack's version")
    page.context.close()
    st1 = get_json(device + "/api/status")
    print(f"  device across the flash-UI checks: misses {st0['deadline_misses']} -> {st1['deadline_misses']} "
          f"(>=5 ms steady {st1['misses'].get('steady', '-')}, switch {st1['misses'].get('switch', '-')}), "
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
        check(page.locator("#now-skip").count() == 1, "device now: the view renders")
        wait_lit(page, "#now-panel", "device now", 40)
        st0 = get_json(base + "/api/status")
        check(all(k in st0["now_playing"] for k in ("elapsed_ms", "frame")) and "t_ms" in st0,
              "device now: status has now_playing.elapsed_ms/frame and t_ms")
        secs = float(os.environ.get("DRIFT_S", "60"))
        drifts, rtts = drift_stats(page, secs)
        st1 = get_json(base + "/api/status")
        print("  " + summarise(f"device drift over {secs:.0f} s", drifts, rtts))
        print(f"  device over that minute: misses {st0['deadline_misses']} -> {st1['deadline_misses']} "
              f"(>=5 ms steady {st1['misses'].get('steady', '-')}, switch {st1['misses'].get('switch', '-')}), "
              f"worst {st1['worst_lateness_ms']} ms, files {st0['gifs_started']} -> {st1['gifs_started']}")
        check(len(drifts) > secs, f"device now: the preview kept syncing ({len(drifts)} syncs)")
        no_leaks(page, "device now")
        shot(page, "device-now", "laptop")
        open_library(page, exact=False)
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
            page.wait_for_function(f"(document.querySelector('#lib-cap').dataset.shown || '').endsWith({json.dumps(name[:-4] + '.gif')})", timeout=40000)
            wait_lit(page, "#lib-panel", f"device library {gif}", 40)
            fetched = [(u.split(base, 1)[-1], s) for u, s in reqs if "/cache/" in u or "/gifs/" in u]
            check((pick, 200) in fetched and not any(u == gif for u, _ in fetched),
                  f"device library: {gif} previewed from its prepared copy ({fetched})")
            check((page.get_attribute("#lib-cap", "data-shown") or '').endswith(name[:-4] + ".gif"), f"device library: the preview plays the GIF ({page.get_attribute('#lib-cap', 'data-shown')})")
            no_leaks(page, "device library")
        # The cache count.
        t0 = time.time()
        r = cache_ready(base, 180)
        check(r["state"] == "ready" and r["prepared"] is not None,
              f"device cache: /api/cache {r['prepared']} of {r['total']} prepared (count {r['count_ms']} ms, ready after {time.time() - t0:.1f} s)")
        goto_view(page, "settings/device")
        page.wait_for_selector("#st-facts dt", timeout=20000)
        check(page.input_value("#st-hostname") != "", "device settings: config GET rendered")
        cfg = get_json(base + "/api/config")
        check(cfg.get("transition") and page.input_value("#st-trans-ms") == str(cfg.get("transition_ms")),
              f"device settings: transition {cfg.get('transition')}:{cfg.get('transition_ms')} shown")
        st = get_json(base + "/api/status")
        facts = page.inner_text("#st-facts") + page.inner_text("#st-about")
        check(st.get("index_state") in facts and (st.get("www_version") or "–") in facts,
              f"device settings: index_state {st.get('index_state')}, www_version {st.get('www_version')} shown")
        # Follow RePlayOS: read-only here -- the card renders and the API
        # shapes are present, but nothing is enabled or saved.
        follow_keys = ("follow", "follow_host", "follow_code_set", "attract_set")
        check(all(k in cfg for k in follow_keys), f"device config: follow keys present ({sorted(cfg)})")
        check("context" in st, f"device status: context present ({st.get('context')})")
        page.wait_for_selector("#fo", timeout=10000)
        check(page.locator("#fo-on").count() == 1 and page.locator("#fo-host").count() == 1,
              "device settings: Follow RePlayOS group renders")
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
        no_effects_checks(page, base, False)
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
        sets_do(page, '/sets/smoke.txt', "delete")
        page.wait_for_selector("#sets-list [data-path='/sets/smoke.txt']", state="detached", timeout=30000)
        listing = get_json(base + "/api/files?dir=/sets")
        check(not any(e["name"].startswith("smoke.txt") for e in listing["entries"]), "device sets: smoke deleted")
        page.context.close()
        phone = new_page(browser, "phone", errors)
        phone.goto(base + "/")
        phone.wait_for_function("document.querySelector('#now-path') && document.querySelector('#now-path').textContent.startsWith('/')", timeout=30000)
        wait_lit(phone, "#now-panel", "device phone now", 40)
        shot(phone, "device-now", "phone")
        goto_view(phone, "settings/device")
        phone.wait_for_selector("#st-facts dt", timeout=20000)
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


# The www update (the beta.13 -> beta.14 failure): a browser that loaded one
# build must run the next build's wasm modules after a plain reload, never
# the ones it kept. Each tree is the built app marked: a custom section in
# every wasm (`opendmd-test-build`, "<dir>:<label>": inert, read back from
# the compiled module) and a line in every wasm-pack glue that records its
# label when it runs; a different version.txt; js/build.js written again
# (tools/www-build-id.py); the .gz beside each changed file made again (the
# mock serves those first, as the pack does).
WWW_PKGS = {"pkg": "opendmd_web", "pkg-psi": "opendmd_psi_web", "pkg-art": "opendmd_art_web"}
WASM_SPY = """(() => {
  const rec = (m) => { try { const s = WebAssembly.Module.customSections(m, 'opendmd-test-build');
    (globalThis.__odmWasm ??= []).push(s.length ? new TextDecoder().decode(s[0]) : 'unmarked'); } catch (e) {} };
  const st = WebAssembly.instantiateStreaming, it = WebAssembly.instantiate;
  if (st) WebAssembly.instantiateStreaming = async function (...a) { const r = await st.apply(this, a); rec(r.module); return r; };
  WebAssembly.instantiate = async function (...a) { const r = await it.apply(this, a);
    if (r instanceof WebAssembly.Instance) { if (a[0] instanceof WebAssembly.Module) rec(a[0]); } else if (r && r.module) rec(r.module); return r; };
})();"""


def leb128(n: int) -> bytes:
    out = bytearray()
    while True:
        b, n = n & 0x7F, n >> 7
        out.append(b | (0x80 if n else 0))
        if not n:
            return bytes(out)


def mark_www(src: Path, dst: Path, label: str) -> None:
    shutil.rmtree(dst, ignore_errors=True)
    shutil.copytree(src, dst)
    changed = []
    for d, name in WWW_PKGS.items():
        wasm = dst / d / f"{name}_bg.wasm"
        name_b, payload = b"opendmd-test-build", f"{d}:{label}".encode()
        body = leb128(len(name_b)) + name_b + payload
        wasm.write_bytes(wasm.read_bytes() + b"\x00" + leb128(len(body)) + body)
        glue = dst / d / f"{name}.js"
        glue.write_text(glue.read_text() + f"\n(globalThis.__odmBuilds ??= []).push('{d}:{label}');\n")
        changed += [wasm, glue]
    (dst / "version.txt").write_text(f"v1.0.0-test-{label}\n")
    if (dst / "js" / "build.js").is_file():  # (main's app had no build id)
        subprocess.run(["uv", "run", "--quiet", str(REPO / "tools" / "www-build-id.py"), str(dst)], check=True, stdout=subprocess.DEVNULL)
        changed.append(dst / "js" / "build.js")
    for f in changed:
        gz = f.with_name(f.name + ".gz")
        if gz.exists():
            gz.write_bytes(gzip.compress(f.read_bytes(), 9, mtime=0))


def www_update_pass(browser) -> None:
    print("\n== www update: a plain reload after the web app is replaced runs the new wasm modules")
    work = OUT / "www-update"
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir(parents=True)
    old, new, live = work / "old", work / "new-build", work / "live"
    mark_www(www_snapshot(), old, "old")
    mark_www(www_snapshot(), new, "new-build")
    shutil.copytree(old, live)
    overlay = work / "overlay"
    overlay.mkdir()
    proc, base = start_mock(["--root", str(CARD), "--overlay", str(overlay), "--www", str(live)])
    errors: list[str] = []
    try:
        # The mock serves the web app as the firmware does: no-cache, an ETag, 304s.
        st, h = status_of(urllib.request.Request(base + "/pkg-psi/opendmd_psi_web_bg.wasm"))
        tag = h.get("ETag")
        st2, _ = status_of(urllib.request.Request(base + "/pkg-psi/opendmd_psi_web_bg.wasm", headers={"If-None-Match": tag or '"none"'}))
        check(st == 200 and h.get("Cache-Control") == "no-cache" and bool(tag) and st2 == 304,
              f"www update: a wasm file goes out no-cache with an ETag, 304 when the browser has it ({st} {h.get('Cache-Control')} {tag} -> {st2})")
        # The first build is served as 1.0.0-beta.13 served it (the wasm and
        # pkg/ kept an hour, no ETag): what a browser holds when the update comes.
        check(get_json(base + "/mock/www-cache?legacy=1")["legacy"], "www update: the first build served with beta.13's headers")
        # A context of its own, without new_page's route for the art index:
        # Playwright turns the HTTP cache off in a context with routes, and
        # the browser's cache is what this is about.
        page = browser.new_context(viewport={"width": 1280, "height": 800}).new_page()
        page.on("console", lambda m: m.type == "error" and errors.append(f"console: {m.text}"))
        page.on("pageerror", lambda e: errors.append(f"pageerror: {e}"))
        page.add_init_script(WASM_SPY)

        def loaded(what: str) -> tuple[list[str], list[str]]:
            """Load the app's three wasm modules as it does (the Formulas view,
            then the Studio's module); what the glue and the wasm said."""
            try:
                page.wait_for_function("document.querySelector('#now-path')?.textContent.startsWith('/')", timeout=20000)
                goto_view(page, "formulas")
                page.wait_for_function("(globalThis.__odmBuilds || []).some(b => b.startsWith('pkg-psi:'))", timeout=20000)
                page.evaluate("import('./js/artmod.js').then(m => m.art())")
                page.wait_for_function("['pkg:', 'pkg-psi:', 'pkg-art:'].every(d => (globalThis.__odmWasm || []).some(b => b.startsWith(d)))", timeout=20000)
            except PlaywrightTimeoutError as e:
                check(False, f"www update: {what}: the app's three wasm modules loaded ({str(e).splitlines()[0]})")
            return page.evaluate("[globalThis.__odmBuilds || [], globalThis.__odmWasm || []]")

        page.goto(base + "/")
        glue, wasm = loaded("first load")
        check(glue and wasm and all(b.endswith(":old") for b in glue + wasm),
              f"www update: the first load runs the first build (glue {sorted(set(glue))}, wasm {sorted(set(wasm))})")
        # The update: every file replaced by the next build's, then a plain reload.
        goto_view(page, "now")  # (the address the reload reloads)
        shutil.rmtree(live)
        shutil.copytree(new, live)
        check(not get_json(base + "/mock/www-cache?legacy=0")["legacy"], "www update: the new build served with today's headers")
        page.reload()
        glue, wasm = loaded("after the update")
        dirs = {b.split(":")[0] for b in wasm}
        check(dirs == set(WWW_PKGS) and set(glue) >= {f"{d}:new-build" for d in WWW_PKGS}
              and all(b.endswith(":new-build") for b in glue + wasm),
              f"www update: after a plain reload every wasm module and its glue is the new build's (glue {sorted(set(glue))}, wasm {sorted(set(wasm))})")
        page.context.close()
    finally:
        proc.terminate()
        proc.wait()
    check(not errors, f"www update: zero console/page errors {errors[:3]}")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--device", help="also run the read-mostly pass against this device URL")
    ap.add_argument("--device-only", action="store_true")
    ap.add_argument("--www-update-only", action="store_true", help="only the www update pass (a reload after the app is replaced)")
    args = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True, args=[f"--host-resolver-rules={LOCAL_RULE}"])
        if args.www_update_only:
            www_update_pass(browser)
        elif not args.device_only:
            mock_pass(browser)
            sd_unreadable_pass(browser)
            sd_slowed_pass(browser)
            www_update_pass(browser)
        if args.device:
            device_pass(browser, args.device.rstrip("/"))
        browser.close()
    mock_errors()
    print(f"\n{'FAILED: ' + str(len(failures)) if failures else 'all checks passed'}")
    return 1 if failures else 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception:
        mock_errors()  # a crash on a 500: the mock's side of it
        raise
