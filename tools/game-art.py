# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright>=1.45", "pillow"]
# ///
"""Panel GIFs of a few well-known games' title screens, made the way a user
makes them: Create Art's Game art search (the art index as the site
publishes it, opendmd-web/art-index/), a game's title screen from
libretro-thumbnails (raw.githubusercontent.com, fetched by the page), then
Save to the card, on tools/mock-device.py over the demo card. For the
screenshots only (tools/screenshots.py).

    uv run tools/game-art.py [--card target/demo-card] [--out target/game-art]

Writes OUT/gifs/<Category>/<name>.gif, named by the card's grammar as
Create Art names them (MD_Sonic_the_Hedgehog_3_01.gif), and OUT/done.json;
a run with the same list and every GIF in place does nothing. These GIFs
are made from third-party title screens: they stay in target/ (gitignored)
and are never committed, put on a card that is published or hosted. Only
screenshots of the web app showing them are published, credited
("Game title screens from libretro-thumbnails, made into panel GIFs with
Create Art"). With no network a game is skipped, and with none made the
screenshots use the Kenney sprites (exit status 0 either way).
"""

import argparse
import hashlib
import json
import shutil
import sys
import tempfile
import urllib.parse
from pathlib import Path

from playwright.sync_api import Error as PlaywrightError, sync_playwright

ROOT = Path(__file__).resolve().parents[1]
import importlib.util  # noqa: E402

_spec = importlib.util.spec_from_file_location("screenshots", ROOT / "tools/screenshots.py")
shots = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(shots)

INDEX = ROOT / "opendmd-web/art-index"
# platform, what to search for, the libretro-thumbnails name of the picture
# to pick (its Named_Titles), the title to save it as, the category, and
# the Frame step's choices as a user would make them (scale: pixels, half
# or fit, "" for Create Art's own; motion: still, pan or scroll; where on
# the picture to tap to put the window there, as fractions, or None).
GAMES = [
    ("MD", "sonic the hedgehog 3", "Sonic The Hedgehog 3 (USA)", "Sonic the Hedgehog 3", "Consoles", "", "still", None),
    ("MD", "streets of rage 2", "Streets of Rage 2 (USA)", "Streets of Rage 2", "Consoles", "half", "still", (0.5, 0.3)),
    ("SNES", "super mario world", "Super Mario World (USA)", "Super Mario World", "Consoles", "half", "still", (0.5, 0.36)),
    ("NES", "super mario bros. 3", "Super Mario Bros. 3 (USA)", "Super Mario Bros. 3", "Consoles", "half", "still", (0.5, 0.45)),
    ("GB", "tetris", "Tetris (World) (Rev 1)", "Tetris", "Consoles", "half", "still", None),
    ("ARCADE", "street fighter ii world warrior world", "Street Fighter II_ The World Warrior (World 910522)", "Street Fighter II", "Arcade", "half", "still", None),
    ("ARCADE", "donkey kong", "Donkey Kong (US set 1)", "Donkey Kong", "Arcade", "half", "still", None),
]
CATEGORIES = sorted({g[4] for g in GAMES})


def recipe() -> str:
    return hashlib.sha256(json.dumps(GAMES).encode()).hexdigest()[:16]


def made(out: Path) -> list[str]:
    """The GIFs a finished run made, or [] when the list changed or one is gone."""
    try:
        d = json.loads((out / "done.json").read_text())
    except (OSError, ValueError):
        return []
    files = d.get("files", [])
    ok = d.get("recipe") == recipe() and all((out / "gifs" / f).is_file() for f in files)
    return files if ok else []


def make_one(page, overlay: Path, game) -> str | None:
    plat, query, pick, title, cat, scale, motion, at = game
    # (A picture in folds Find: open it again.)
    if page.is_visible("#st-change"):
        page.click("#st-change")
    page.click("#st-src [data-v=gameart]", force=True)
    page.select_option("#find-platform", plat)
    page.fill("#st-q", query)
    page.evaluate("document.querySelectorAll('#find-grid .find-hit').forEach(e => e.remove())")
    page.click("#st-go")
    # (find.js names a picture by its address: the libretro file name.)
    try:
        page.wait_for_selector("#find-grid .find-hit[data-kind=Named_Titles]", timeout=30000)
    except PlaywrightError:
        print(f"  {plat} {pick}: no title screens found", file=sys.stderr)
        return None
    ids = page.eval_on_selector_all("#find-grid .find-hit[data-kind=Named_Titles]", "els => els.map(e => e.dataset.id)")
    match = [i for i in ids if urllib.parse.unquote(i.rsplit("/", 1)[1]) == pick + ".png"]
    if not match:
        print(f"  {plat} {pick}: not among the results", file=sys.stderr)
        return None
    hit = page.locator(f"#find-grid .find-hit[data-id={json.dumps(match[0])}]")
    page.evaluate("delete document.querySelector('#studio').dataset.frames")
    hit.first.click()
    page.wait_for_function("document.querySelector('#studio').dataset.frames", timeout=30000)
    if scale:
        page.click(f"#st-fit [data-v={scale}]")
        page.wait_for_timeout(400)
    if page.is_visible(f"#st-motion [data-v={motion}]"):
        page.click(f"#st-motion [data-v={motion}]")
        page.wait_for_timeout(400)
    if at:  # a tap on the picture puts the window there
        page.evaluate("""([fx, fy]) => { const v = document.querySelector('#st-view'), r = v.querySelector('canvas').getBoundingClientRect();
            v.dispatchEvent(new MouseEvent('click', { bubbles: true, clientX: r.left + fx * r.width, clientY: r.top + fy * r.height })); }""", list(at))
        page.wait_for_timeout(400)
    page.select_option("#st-plat", plat)
    page.select_option("#st-cat", cat)
    page.fill("#st-title", title)
    page.wait_for_function(f"document.querySelector('#studio').dataset.title === {json.dumps(title)}", timeout=10000)
    before = {p for p in overlay.rglob("*.gif")}
    page.evaluate("delete document.querySelector('#studio').dataset.saved")
    page.click("#st-save")
    page.wait_for_function("document.querySelector('#studio').dataset.saved", timeout=120000)
    page.wait_for_timeout(500)
    new = [p for p in overlay.glob(f"gifs/{cat}/*.gif") if p not in before]
    if len(new) != 1:
        print(f"  {plat} {pick}: saved {len(new)} files", file=sys.stderr)
        return None
    page.keyboard.press("Escape")  # (the offer to add it to the playlist)
    return new[0].relative_to(overlay / "gifs").as_posix()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--card", type=Path, default=ROOT / "target/demo-card")
    ap.add_argument("--out", type=Path, default=ROOT / "target/game-art")
    a = ap.parse_args()
    if made(a.out):
        print(f"{a.out}: {len(made(a.out))} game title GIFs, as before")
        return 0
    shutil.rmtree(a.out / "gifs", ignore_errors=True)
    overlay = Path(tempfile.mkdtemp(prefix="opendmd-game-art-"))
    for c in CATEGORIES:
        (overlay / "gifs" / c).mkdir(parents=True)
    proc, base = shots.start_mock(a.card, overlay)
    files: list[str] = []
    try:
        with sync_playwright() as pw:
            browser = pw.chromium.launch()
            page = shots.page_for(browser, shots.LAPTOP)
            # The art index as the site publishes it (the committed one).
            page.route("**/art-index/v1/*.json", lambda r: r.fulfill(
                path=str(INDEX / Path(urllib.parse.urlsplit(r.request.url).path).name), content_type="application/json",
                headers={"Access-Control-Allow-Origin": "*"}))
            page.goto(base + "/#create")
            page.wait_for_selector("#st-src", timeout=30000)
            for g in GAMES:
                try:
                    f = make_one(page, overlay, g)
                except PlaywrightError as e:  # no network, most likely
                    print(f"  {g[0]} {g[2]}: {' / '.join(str(e).splitlines()[:4])[:300]}", file=sys.stderr)
                    f = None
                if f:
                    (a.out / "gifs" / f).parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(overlay / "gifs" / f, a.out / "gifs" / f)
                    files.append(f)
                    print(f"  {a.out / 'gifs' / f}")
            browser.close()
    finally:
        proc.terminate()
        shutil.rmtree(overlay, ignore_errors=True)
    if len(files) == len(GAMES):
        (a.out / "done.json").write_text(json.dumps({"recipe": recipe(), "files": files}, indent=1) + "\n")
    print(f"{a.out}: {len(files)} of {len(GAMES)} game title GIFs" + ("" if files else " (the screenshots use the Kenney sprites)"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
