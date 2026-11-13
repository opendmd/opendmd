# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright>=1.45", "pillow"]
# ///
"""The README's, the guide's and the site's screenshots of the web app,
taken in headless Chromium against tools/mock-device.py over the demo card
(and tools/mock-replay.py, a fake RePlayOS, for Follow).

    uv run tools/screenshots.py [--card target/demo-card] [--out docs/images] [--game-art target/game-art]

(`make screenshots` builds the app, the simulator and the demo card
first.) Writes, at 1280x800: now.png, formulas.png and playlists.png;
create-art.png (1280 wide, the whole page); at 2x, the part that matters
in a 1280x800 window (the site's bands): library.png (Select, the playlist
menu open), now-follow.png (Now while following RePlayOS, as a game
plays), settings-look.png (Settings › Panel) and settings-schedule.png
(Settings › Schedule: a Christmas rule, an evening rule, night mode from
sunset to sunrise); and at 2x, phone-now.png (390x700) and
phone-library.png (390x844).

The card is tools/demo-card.py's: Formulas recorded as GIFs and the CC0
Kenney sprites, nothing from the DMDoS pack. On top of it, on a throwaway
overlay, go a few more of OpenDMD's own Formulas (opendmd-web/formulas/)
as the card's Formulas, in the playlists beside the GIFs; a folder of
GIFs recorded from our Formulas, which is what Now's panel and the
Library show (Now mirrors GIFs: a Formula shows its name there); and a
recording of Rift patrol filed as a Mega Drive game's GIF, which Follow
plays while the fake RePlayOS runs "Rift Patrol" (a made-up game: nothing
in a shot is anyone else's art).

panels.json, beside the shots, says where the panel is in the ones the
site's home page plays a Formula over (tools/site.py, home.js): the
picture's name -> [left, top, width, height] in hundredths of the
picture, the Formula on it, the web app's pixels a LED there, and the
frame it shows (ms into the Formula, at its seed).

`--game-art DIR` (tools/game-art.py's output; `make screenshots
GAME_ART=1`) casts game title screens from libretro-thumbnails, made
into panel GIFs with Create Art, instead: not what the published shots
show, and such shots need the credit line (CLAUDE.md, "Content
licensing"); panels.json is then left alone. No shot may show a card
path, "odm", "bucket" or "cache". The card is left as it was.
Needs `make web` and, once, `uv run --with playwright playwright install
chromium`.
"""

import argparse
import json
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

from PIL import Image
from playwright.sync_api import Page, sync_playwright

ROOT = Path(__file__).resolve().parents[1]
KENNEY_WALK = "/gifs/Kenney/OTHER_Pixel_Platformer_Walk_01_kenney.gif"
KENNEY_PAN = "/gifs/Kenney/OTHER_Pixel_Platformer_Pan_01_kenney.gif"
KENNEY_SCROLL = "/gifs/Kenney/OTHER_Pixel_Platformer_Scroll_01_kenney.gif"
PHONE = {"viewport": {"width": 390, "height": 844}, "device_scale_factor": 2, "is_mobile": True, "has_touch": True}
LAPTOP = {"viewport": {"width": 1280, "height": 800}}


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def wait_port(proc: subprocess.Popen, port: int, what: str) -> None:
    for _ in range(150):
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                return
        except OSError:
            time.sleep(0.1)
    proc.kill()
    sys.exit(f"{what} did not come up")


def start_mock(card: Path, overlay: Path) -> tuple[subprocess.Popen, str]:
    port = free_port()
    proc = subprocess.Popen(["uv", "run", str(ROOT / "tools/mock-device.py"), "--port", str(port), "--quiet",
                             "--root", str(card), "--overlay", str(overlay), "--dwell", "600"],
                            stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT)
    wait_port(proc, port, "mock-device.py")
    return proc, f"http://127.0.0.1:{port}"


def start_replay(token: str, timeline: Path | None = None) -> tuple[subprocess.Popen, str, int]:
    """A fake RePlayOS, parked in its menu until told otherwise (`timeline`:
    its steps, else mock-replay.py's own: the menu, then a game)."""
    port = free_port()
    proc = subprocess.Popen(["uv", "run", str(ROOT / "tools/mock-replay.py"), "--port", str(port), "--token", token,
                             "--speed", "0.01", "--quiet", *(["--timeline", str(timeline)] if timeline else [])],
                            stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT)
    wait_port(proc, port, "mock-replay.py")
    return proc, f"http://127.0.0.1:{port}", port


def call(base: str, method: str, path: str, body=None) -> None:
    data = b"" if body is None else json.dumps(body).encode()
    req = urllib.request.Request(base + path, data=data, method=method, headers={"Content-Type": "application/json"})
    urllib.request.urlopen(req, timeout=10).read()


def page_for(browser, size: dict) -> Page:
    # bypass_csp: Playwright's wait_for_function evaluates strings, which the
    # app's Content-Security-Policy refuses (as in tools/web-smoke.py). The
    # banner asking for an API key is put off ("Not now"), as a reader would.
    ctx = browser.new_context(**size, bypass_csp=True)
    ctx.add_init_script("try { sessionStorage.setItem('odmd.keynag', 'later'); } catch {}")
    return ctx.new_page()


def status(base: str) -> dict:
    with urllib.request.urlopen(base + "/api/status", timeout=10) as r:
        return json.load(r)


def reboot(base: str) -> None:
    """A restart (Now's count and Recently played start again), then the first GIF."""
    call(base, "POST", "/api/reboot")
    for _ in range(100):
        time.sleep(0.2)
        try:
            if status(base)["now_playing"]["path"]:
                time.sleep(0.8)
                return
        except (OSError, KeyError):
            pass
    sys.exit("the mock did not come back from its reboot")


def skip_to(base: str, path: str, at_least: int) -> None:
    """Skip through the playlist (filling Recently played) until `path` is on."""
    for n in range(40):
        if n >= at_least and status(base)["now_playing"]["path"] == path:
            time.sleep(1.0)
            return
        call(base, "POST", "/api/skip")
        time.sleep(0.6)
    sys.exit(f"never reached {path}")


def wait_lit(page: Page, canvas: str, timeout_s: float = 20) -> None:
    """Until the preview shows something (and a moment more for the glow)."""
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        n = page.evaluate("""(sel) => {
            const c = document.querySelector(sel);
            if (!c || !c.width) return 0;
            const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
            let n = 0;
            for (let i = 0; i < d.length; i += 4) if (d[i] > 60 || d[i+1] > 60 || d[i+2] > 60) n++;
            return n;
        }""", canvas)
        if n > 200:
            page.wait_for_timeout(600)
            return
        page.wait_for_timeout(250)
    sys.exit(f"{canvas}: the preview never lit")


def go(page: Page, view: str) -> None:
    page.evaluate(f"location.hash = '#{view}'")
    page.wait_for_selector(f"#view-{view.split('/')[0]}.on")
    page.wait_for_timeout(500)


def folder(page: Page, cat: str) -> None:
    """The Library with the folder `cat` alone open, at the top (`#library/<cat>`)."""
    page.evaluate(f"location.hash = '#library/' + encodeURIComponent({json.dumps(cat)})")
    page.wait_for_function(f"(t => t && t.open.has({json.dumps(cat)}) && t.rows.some(r => r.path))(document.querySelector('#lib-list')?._tree)", timeout=20000)


def select(page: Page, paths: list[str]) -> None:
    """The Library's Select, with `paths` ticked (a tap on each, as a user picks them)."""
    page.click("#lib-select")
    for p in paths:
        page.click(f"#lib-list .vrow[data-path={json.dumps(p)}]")
        page.wait_for_timeout(150)
    page.wait_for_function(f"document.querySelector('#lib-selbar b')?.textContent === '{len(paths)}'", timeout=10000)
    page.wait_for_timeout(400)


def now_ready(page: Page, path: str) -> None:
    page.wait_for_function(f"document.querySelector('#now-path')?.textContent === {json.dumps(path)}", timeout=20000)
    wait_lit(page, "#now-panel")
    page.wait_for_selector("#now-recent > div", timeout=10000)
    page.wait_for_timeout(1500)


def shot(page: Page, out: Path, name: str, full: bool = False) -> None:
    page.evaluate("document.activeElement?.blur?.(); document.querySelectorAll('#toasts .toast').forEach(t => t.remove())")
    page.mouse.move(0, 0)
    page.wait_for_timeout(200)
    page.screenshot(path=str(out / name), full_page=full)
    print(f"  {out / name}")


def clip(page: Page, out: Path, name: str, parts: str, pad: int = 12) -> None:
    """A shot of just the elements the JS expression `parts` gives (their
    bounding box, `pad` px of the page around it), at the page's scale."""
    page.evaluate("document.activeElement?.blur?.(); document.querySelectorAll('#toasts .toast').forEach(t => t.remove())")
    page.mouse.move(0, 0)
    page.wait_for_timeout(200)
    box = page.evaluate(f"""(pad) => {{
        const rs = ({parts}).filter(Boolean).map(e => e.getBoundingClientRect());
        const x = Math.min(...rs.map(r => r.left)) - pad, y = Math.min(...rs.map(r => r.top)) - pad;
        return {{x: x + scrollX, y: y + scrollY, width: Math.max(...rs.map(r => r.right)) + pad - x, height: Math.max(...rs.map(r => r.bottom)) + pad - y}};
    }}""", pad)
    page.screenshot(path=str(out / name), full_page=True, clip=box)
    print(f"  {out / name}")


FORMULAS = ROOT / "opendmd-web/formulas"
# Our own cast. Now's panel mirrors GIFs (a Formula in a playlist shows its
# name there, and only the display draws it), so what plays in the shots
# are GIFs recorded from our Formulas, as the demo card's Demoscene folder
# is: a folder of them made here, on the overlay (ANIMATIONS: file stems in
# opendmd-web/formulas/). The card has Formulas in its playlists too:
# bundled ones by name, and a few of its own (their files, copied to the
# overlay's /formulas/).
FOLDER = "Animations"
ANIMATIONS = ("copper-bars", "endless-road", "fire", "neon-city", "plasma", "starfield", "tunnel", "wormhole-run")
CARD_FORMULAS = ("fire", "plasma", "wormhole-run")


def gif_of(stem: str) -> str:
    return f"/gifs/{FOLDER}/{stem.replace('-', '_').title()}_01.gif"


PHONE_NOW = gif_of("neon-city")
NOW = gif_of("endless-road")
# Follow: the game on the fake box, and the GIF of it (a recording of Rift
# patrol, filed under the console as a GIF made for that game would be).
FOLLOW_GIF = "/gifs/Games/MD_Rift_Patrol_01.gif"
FOLLOW_GAME = {"view": "game_play", "secs": 3600, "system": "sega_smd", "game_name": "Rift Patrol (World)",
               "game_file": "/media/nvme/roms/sega_smd/Rift Patrol (World).md", "core_file": "genesis_plus_gx_libretro.so",
               "core_info": "Genesis Plus GX"}


def formula_src(stem: str) -> str:
    """A Formula's file without its header (author, licence, bundle)."""
    lines = (FORMULAS / f"{stem}.psi").read_text(encoding="utf-8").split("\n")
    while lines and lines[0].startswith(("-- author:", "-- licence:", "-- license:", "-- bundle")):
        lines.pop(0)
    return "\n".join(lines)


# Where record() starts, and its step: frame i of a recording is the
# Formula `RECORD_AT + i * RECORD_STEP` ms in, at seed 0 (the simulator's
# first open, with no clock: opendmd-core fx_psi `next_seed`).
RECORD_AT, RECORD_STEP = 2000, 40


def record(sim: Path, card: Path, stem: str, out: Path, frames: int = 75, at: int = RECORD_AT) -> None:
    """A Formula as a panel GIF (the simulator's raw canvas, a frame at a
    time, as tools/demo-card.py records the Demoscene ones)."""
    out.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        psi, png = Path(tmp) / "f.psi", Path(tmp) / "f.png"
        psi.write_text(formula_src(stem))
        shots = []
        for i in range(frames):
            subprocess.run([sim, "--root", card, "--formula", psi, "--screenshot-canvas", png, "--screenshot-at", str(at + i * RECORD_STEP)],
                           check=True, capture_output=True)
            shots.append(Image.open(png).convert("RGB").quantize(64, dither=Image.Dither.NONE))
    shots[0].save(out, save_all=True, append_images=shots[1:], duration=RECORD_STEP, loop=0)


def own_cast(card: Path, overlay: Path, sim: Path) -> dict:
    """Who plays what in the shots: recordings of our own Formulas
    (ANIMATIONS, and the card's Demoscene folder), Formulas themselves and
    the sprites. All files mixes them, and a Favourites playlist plays."""
    (overlay / "formulas").mkdir(exist_ok=True)
    for stem in CARD_FORMULAS:
        shutil.copyfile(FORMULAS / f"{stem}.psi", overlay / "formulas" / f"{stem}.psi")
    for stem in ANIMATIONS:
        record(sim, card, stem, overlay / gif_of(stem).lstrip("/"))
    record(sim, card, "rift-patrol", overlay / FOLLOW_GIF.lstrip("/"))
    g = {stem: gif_of(stem) for stem in ANIMATIONS}
    files = ["@formula:starfield", PHONE_NOW, KENNEY_WALK, NOW, g["plasma"], KENNEY_PAN, FOLLOW_GIF, "/formulas/plasma.psi",
             g["fire"], "@formula:time-gate", KENNEY_SCROLL, "/formulas/wormhole-run.psi", g["tunnel"], "/formulas/fire.psi",
             g["copper-bars"], g["wormhole-run"], g["starfield"]]
    (overlay / "filelist.txt").write_text("\n".join(files) + "\n")
    (overlay / "sets").mkdir(exist_ok=True)
    # (The walk's GIF opens on black and wipes in: its thumbnail is a sliver; the pan's is the scene.)
    (overlay / "sets/Favourites.txt").write_text("\n".join([NOW, PHONE_NOW, "@formula:starfield", FOLLOW_GIF, KENNEY_PAN, "/formulas/fire.psi", g["plasma"], g["tunnel"]]) + "\n")
    return {"phone_now": PHONE_NOW, "now": NOW, "set": "Favourites", "folder": FOLDER,
            "ticks": [g["endless-road"], g["fire"]], "follow": FOLLOW_GIF, "game": [FOLLOW_GAME],
            "plays": {"phone-now.png": "neon-city.psi", "now-follow.png": "rift-patrol.psi"},
            "recorded": {"phone-now.png": PHONE_NOW, "now-follow.png": FOLLOW_GIF}}


def panel_at(page: Page, parts: str | None, pad: int = 12) -> list[float]:
    """Where Now's panel is in a shot: [left, top, width, height] in
    hundredths of the picture (the window, or clip()'s box round `parts`)."""
    return page.evaluate(f"""(pad) => {{
        const p = document.querySelector('#now-panel').getBoundingClientRect();
        const rs = {parts or "null"};
        let x = 0, y = 0, w = innerWidth, h = innerHeight;
        if (rs) {{
            const b = rs.filter(Boolean).map(e => e.getBoundingClientRect());
            x = Math.min(...b.map(r => r.left)) - pad; y = Math.min(...b.map(r => r.top)) - pad;
            w = Math.max(...b.map(r => r.right)) + pad - x; h = Math.max(...b.map(r => r.bottom)) + pad - y;
        }}
        return [(p.left - x) / w, (p.top - y) / h, p.width / w, p.height / h].map(v => Math.round(v * 1e5) / 1e3);
    }}""", pad)


def led_scale(page: Page) -> int:
    """Now's panel's pixels a LED (the web app's renderer draws at 6..10)."""
    return page.evaluate("document.querySelector('#now-panel').width / 128")


def frame_shown(png: Path, at: list[float], gif: Path) -> int:
    """Which of `gif`'s frames (a recording: record()) the shot `png` shows
    on its panel (`at`, panel_at's box): the one nearest at the LEDs' centres."""
    shot = Image.open(png).convert("RGB")
    l, t, w, h = (v / 100 for v in at)
    W, H = shot.size
    got = [shot.getpixel((int((l + (x + 0.5) * w / 128) * W), int((t + (y + 0.5) * h / 32) * H))) for y in range(32) for x in range(128)]
    best = (float("inf"), 0)
    im = Image.open(gif)
    for i in range(im.n_frames):
        im.seek(i)
        f = im.convert("RGB")
        d = sum((a - b) ** 2 for p, q in zip(got, (f.getpixel((x, y)) for y in range(32) for x in range(128))) for a, b in zip(p, q))
        best = min(best, (d, i))
    return best[1]


def game_art(art: Path) -> list[str]:
    """tools/game-art.py's GIFs (card paths), in its order, or [] (none made:
    the Kenney sprites play every part)."""
    try:
        files = json.loads((art / "done.json").read_text())["files"]
    except (OSError, ValueError, KeyError):
        files = sorted(p.relative_to(art / "gifs").as_posix() for p in art.glob("gifs/*/*.gif"))
    return ["/gifs/" + f for f in files if (art / "gifs" / f).is_file()]


def cast(games: list[str], overlay: Path, art: Path) -> dict:
    """Who plays what in the shots with the game title GIFs (--game-art):
    they go on the throwaway overlay (never the card, never committed), All
    files is them and the sprites, and a Favourites playlist plays."""
    by = {Path(g).name.split("_01")[0]: g for g in games}
    want = ("MD_Sonic_the_Hedgehog_3", "ARCADE_Street_Fighter_II", "SNES_Super_Mario_World", "MD_Streets_of_Rage_2", "NES_Super_Mario_Bros._3")
    if not all(k in by for k in want):
        sys.exit(f"{art}: not tools/game-art.py's GIFs (uv run tools/game-art.py --card CARD --out {art})")
    for g in games:
        dst = overlay / g.lstrip("/")
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(art / "gifs" / g.removeprefix("/gifs/"), dst)
    first = [by[k] for k in want[:3]]
    rest = [g for g in games if g not in first]
    (overlay / "filelist.txt").write_text("\n".join(first + rest + [KENNEY_WALK, KENNEY_PAN, KENNEY_SCROLL]) + "\n")
    fav = [g for g in games if g not in (by["MD_Streets_of_Rage_2"], by["NES_Super_Mario_Bros._3"])]
    (overlay / "sets").mkdir(exist_ok=True)
    (overlay / "sets/Favourites.txt").write_text("\n".join(fav) + "\n")
    return {"phone_now": by["ARCADE_Street_Fighter_II"], "now": by["SNES_Super_Mario_World"], "set": "Favourites", "folder": "Consoles",
            "ticks": [by["MD_Streets_of_Rage_2"], by["NES_Super_Mario_Bros._3"]], "follow": by["MD_Sonic_the_Hedgehog_3"], "game": None, "plays": None}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--card", type=Path, default=ROOT / "target/demo-card")
    ap.add_argument("--out", type=Path, default=ROOT / "docs/images")
    ap.add_argument("--game-art", type=Path, help="tools/game-art.py's output: game title screens play instead of our own Formulas")
    ap.add_argument("--sim", type=Path, default=ROOT / "opendmd-sim/target/release/opendmd-sim")
    a = ap.parse_args()
    if not (a.card / "index.json").exists():
        sys.exit(f"{a.card}: not a demo card (uv run tools/demo-card.py {a.card})")
    a.out.mkdir(parents=True, exist_ok=True)
    overlay = Path(tempfile.mkdtemp(prefix="opendmd-shots-"))
    # A Christmas list beside the card's own (the schedule offers its rule).
    (overlay / "filelist.xmas").write_text("\n".join((KENNEY_WALK, KENNEY_PAN, KENNEY_SCROLL)) + "\n")
    c = cast(game_art(a.game_art), overlay, a.game_art) if a.game_art else own_cast(a.card, overlay, a.sim)
    print(f"  {'game title screens' if a.game_art else 'our own Formulas'} on the card")
    panels = {}
    proc, base = start_mock(a.card, overlay)
    replay = None
    try:
        # The name the site and README show; the mock's own is opendmd-mock.
        # Now's "N played" counts the GIFs started since boot: a reboot after
        # the settings, then a skip or two through All files (in order).
        call(base, "PUT", "/api/config", {"hostname": "opendmd", "set": "", "shuffle": False})
        reboot(base)
        with sync_playwright() as pw:
            browser = pw.chromium.launch()
            # Now on a phone, then on a laptop: a GIF, the ones before it in
            # Recently played, what comes next.
            # (A shorter phone for Now: its page ends above the fold.)
            phone = page_for(browser, {**PHONE, "viewport": {"width": 390, "height": 700}})
            skip_to(base, c["phone_now"], 1)
            phone.goto(base + "/#now")
            now_ready(phone, c["phone_now"])
            panels["phone-now.png"] = panel_at(phone, None)
            leds = {"phone-now.png": led_scale(phone)}
            shot(phone, a.out, "phone-now.png")
            phone.context.close()
            page = page_for(browser, LAPTOP)
            skip_to(base, c["now"], 1)
            page.goto(base + "/#now")
            now_ready(page, c["now"])
            shot(page, a.out, "now.png")
            # A playlist of the user's playing from here: it heads the playlist menu.
            call(base, "PUT", "/api/config", {"set": c["set"]})
            time.sleep(1.5)
            page.reload()
            # The site's bands: the parts that matter, at twice the scale.
            # Library: a folder, two GIFs ticked, the playlist menu open
            # (a shorter window: the list fills it).
            big = page_for(browser, {"viewport": {"width": 1280, "height": 560}, "device_scale_factor": 2})
            big.goto(base + "/#library")
            folder(big, c["folder"])
            select(big, c["ticks"])
            big.click("#lib-to-set")
            big.wait_for_function("[...document.querySelectorAll('#pl-target [data-path] small')].every(s => s.textContent)", timeout=10000)
            big.wait_for_timeout(300)
            clip(big, a.out, "library.png", "[document.querySelector('#lib-search')?.closest('.card') ?? document.querySelector('#lib-select').closest('.card'), "
                                            "document.querySelector('#lib-list'), document.querySelector('#lib-selbar'), document.querySelector('#pl-target')]")
            big.context.close()
            big = page_for(browser, {**LAPTOP, "device_scale_factor": 2})
            # Formulas: the gallery, the first example (Plasma) in Details.
            go(page, "formulas")
            page.wait_for_selector("#psi-ex .psi-tile", timeout=20000)
            wait_lit(page, "#psi-panel")
            page.wait_for_timeout(800)
            shot(page, a.out, "formulas.png")
            # Playlists: the one playing, open in the editor.
            go(page, "sets")
            page.click(f"#sets-list [data-path='/sets/{c['set']}.txt'] button.pl")
            page.wait_for_selector("#set-entries .vrow[data-path]", timeout=20000)
            page.wait_for_timeout(1200)
            shot(page, a.out, "playlists.png")
            # Settings › Panel: amber with a soft glow.
            call(base, "PUT", "/api/config", {"look": "amber", "glow": "short"})
            big.goto(base + "/#settings/look")
            big.reload()
            big.wait_for_selector("#st-tab-look #st-glow", timeout=20000)
            big.wait_for_selector("#st-power-card *", timeout=10000)
            big.wait_for_timeout(1200)
            clip(big, a.out, "settings-look.png", "[document.querySelector('#st-tab-look > .card')]")
            call(base, "PUT", "/api/config", {"look": "colour", "glow": "off"})
            # Settings › Schedule: Christmas from 1 to 26 December, weekday
            # evenings, and night mode dimming from sunset to sunrise.
            call(base, "PUT", "/api/config", {"tz": "GMT0IST,M3.5.0/1,M10.5.0", "tz_name": "Europe/Dublin", "lat": 53.35, "lon": -6.26})
            call(base, "PUT", "/api/schedule", {
                "rules": [{"days": [], "from": "00:00", "to": "00:00", "set": "filelist.xmas", "dates": ["12-01", "12-26"]},
                          {"days": [1, 2, 3, 4, 5], "from": "18:00", "to": "23:00", "set": "Demoscene"}],
                "night": {"from": "23:30", "to": "07:00", "sun": True, "offset": 30, "mode": "dim", "brightness": 8}})
            big.goto(base + "/#settings/time")
            big.reload()
            big.wait_for_selector("#sc-rules [data-rule='1']", timeout=20000)
            big.wait_for_timeout(1000)
            clip(big, a.out, "settings-schedule.png", "[document.querySelector('#sc')]")
            big.context.close()
            call(base, "PUT", "/api/schedule", {"rules": [], "night": None})
            # Create Art: a still of the Kenney scenery (a frame of the
            # scroll with the most colours), titled and filed under Kenney; the whole page.
            go(page, "create")
            page.wait_for_selector("#st-src", timeout=30000)
            page.click("#st-src [data-v=own]")
            with tempfile.TemporaryDirectory() as tmp:
                f = Path(tmp) / "Pixel_Platformer.png"
                im = Image.open(a.card / KENNEY_SCROLL.lstrip("/"))
                frames = []
                for i in range(im.n_frames):  # the busiest frame: the most colours
                    im.seek(i)
                    frames.append((len(im.convert("RGB").getcolors(4096) or ()), i))
                im.seek(max(frames)[1])
                im.convert("RGB").save(f)
                page.set_input_files("#st-file", str(f))
                page.wait_for_function("document.querySelector('#studio').dataset.frames", timeout=30000)
            page.fill("#st-title", "Pixel Platformer")
            page.select_option("#st-cat", "Kenney")
            page.wait_for_timeout(1500)
            page.evaluate("scrollTo(0, 0)")
            # The rail is fixed: a viewport as tall as the page, not a full-page shot.
            page.set_viewport_size({"width": 1280, "height": page.evaluate("document.documentElement.scrollHeight")})
            page.wait_for_timeout(500)
            shot(page, a.out, "create-art.png")
            page.context.close()

            # The Library on a phone, GIFs ticked.
            phone = page_for(browser, PHONE)
            phone.goto(base + "/#library")
            phone.wait_for_selector("#lib-list", timeout=20000)
            folder(phone, c["folder"])
            select(phone, c["ticks"])
            shot(phone, a.out, "phone-library.png")
            phone.context.close()

            # Following RePlayOS: a game starts on the box, and Now shows its
            # title card with the line under the title.
            call(base, "PUT", "/api/config", {"set": ""})
            reboot(base)
            if c["game"]:  # the menu, then our game
                (overlay / "replay.json").write_text(json.dumps([{"view": "system_list", "secs": 3600}] + c["game"]))
            replay, rbase, rport = start_replay("4321", overlay / "replay.json" if c["game"] else None)
            call(rbase, "POST", "/mock/goto?step=0")
            call(base, "PUT", "/api/config", {"follow": True, "follow_host": f"localhost:{rport}", "follow_code": "4321"})
            page = page_for(browser, {**LAPTOP, "device_scale_factor": 2})
            page.goto(base + "/#now")
            page.wait_for_function("document.querySelector('#now-follow-line')?.textContent.includes('attract')", timeout=30000)
            call(rbase, "POST", "/mock/goto?step=1")
            # The game's own GIF, once the title card has played (it plays
            # the console's GIFs, the game's most often: skip to it).
            page.wait_for_function("document.querySelector('#now-follow-line')?.textContent.includes('matching')", timeout=60000)
            for _ in range(24):
                if status(base)["now_playing"]["path"] == c["follow"]:
                    break
                call(base, "POST", "/api/skip")
                time.sleep(1.0)
            page.wait_for_function(f"document.querySelector('#now-path')?.textContent === {json.dumps(c['follow'])}", timeout=20000)
            wait_lit(page, "#now-panel", 5)
            card = "[document.querySelector('#now-panel').closest('.card')]"
            panels["now-follow.png"] = panel_at(page, card)
            leds["now-follow.png"] = led_scale(page)
            clip(page, a.out, "now-follow.png", card)
            page.context.close()
            browser.close()
        if c["plays"]:
            # The frame each shows (the site's overlay starts there: home.js).
            ms = {k: RECORD_AT + RECORD_STEP * frame_shown(a.out / k, panels[k], overlay / c["recorded"][k].lstrip("/")) for k in c["plays"]}
            (a.out / "panels.json").write_text(json.dumps({k: {"formula": c["plays"][k], "at": panels[k], "led": leds[k], "ms": ms[k], "seed": 0}
                                                           for k in sorted(c["plays"])}, indent=1) + "\n")
            print(f"  {a.out / 'panels.json'}")
    finally:
        proc.terminate()
        if replay:
            replay.terminate()
        shutil.rmtree(overlay, ignore_errors=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
