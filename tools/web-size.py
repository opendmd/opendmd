# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Print the web app's size, raw and gzipped, and enforce the budget.

    uv run tools/web-size.py opendmd-web/www [--pack opendmd-web/www.pack --pack-limit 1310720]

Budget (gzipped bytes — what actually crosses the WiFi, since the firmware
serves `<file>.gz` in preference): the wasm module plus its wasm-bindgen JS
glue (`pkg/`, generated, grows with the export list) <= 116 KB (100 KB
until the live effects, `opendmd_core::fx`, which the wasm player needs:
~11.4 KB gz of renderers and tables; served from flash at ~320 KB/s, the
extra 12 KB is ~40 ms of a page load; 112 KB until the start-up show,
`opendmd_core::boot`, which Settings previews as the `@boot` virtual file
so the page draws exactly what the panel does: +2.4 KB gz); the
hand-written JS + CSS <= 57 KB (40 KB until Follow RePlayOS's Settings card,
which had ~0.6 KB left; 48 KB until the Library's redesign — the Details
panel, the category tabs, Make art as a view —, +8 KB being ~25 ms of a
page load from flash; 56 KB until display names and the platform chips,
2026-10-21, after the panel test and web-ux merges). The Studio ("Make art") is loaded only when it
opens and has lines of its own: its wasm + glue (`pkg-art/`) <= 150 KB, and
`js/studio.js` + `studio.css` <= 24 KB — neither counts against the lines
above. Library's "Live" items and "Effects" (`js/live.js`: the clock, the
scrolling text and the effects' option forms, imported only when either
category is chosen) have a line of their own too, <= 4 KB, and so does
Settings' Start-up card (`js/boot.js`, imported by the Settings view: the
`boot_show` select and the previews of the start-up show), <= 2 KB, and Settings' Power card (`js/power.js`: the Maximum brightness
control and its warning), <= 2 KB, and the Library's Add GIFs and Edit
details dialogs with Delete (`js/details.js`, imported when first used),
<= 8 KB, and so does the Library's search of the web (`js/find.js` +
`find.css`: "GifCities" and "Game art", the results grid, and the game-art
search Make art's "Find art for a game" shares; imported when a web source
is first picked or Make art opens), <= 10 KB, and so does Smooth motion
(`js/smooth.js`: the Library's control, the list, the smoothed copy, Make
art's save of it; imported by a Library preview on a device, a prepare run,
or Make art's switch; counted with the page until 2026-10-01), <= 6 KB,
and so do Settings' Firmware and Security groups (`js/firmware.js`: the
firmware version, "Update from file…" with its progress and wait for the
restart, and the API key switch; imported when Advanced is first open),
<= 4 KB, and so does its "Check for updates" (`js/update.js`: the
release check, the downloads and the installs, imported on the first
click), <= 4 KB, and so does Settings' Panel timing (`js/panel.js`: the
HUB75 timing selects, the trial's Keep/Undo and Reset to defaults;
imported when Advanced is first open, with js/firmware.js), <= 2 KB, and
so does its guided "Test panel timing" (`js/paneltest.js`: the A/B
questions and the save; imported on the first click), <= 2 KB, and
so does Now's card format dialog (`js/card.js`: "Format
card…" for a card the panel can't read, the POST and the wait for it;
imported when first opened), <= 2 KB, and so does the API reference page (`api.html`, rendered from docs/api.md
by tools/api-page.py, opened from Settings -> About in its own tab), <=
13 KB (12 KB until the Security and Firmware update sections, 2026-10-02;
the page is only fetched when opened from Settings → About). The name index for "Find art
for a game" (`art/*.json`, one file
fetched per search) is printed, with no budget. Raw sizes are printed for
information. A file with no `.gz` beside it counts at its raw size. Exits 1
when over budget.

With `--pack`, one more line: the www pack `make web` built from this
directory (what the device keeps in its `www` flash partition, served from
flash), its size against the partition — "pack: N B of 1,280 KB". The
budgets above are about load time over WiFi, not flash; the partition limit
is enforced by the packer itself (`opendmd-pack www --max-bytes`).
"""
import argparse
import sys
from pathlib import Path

# 100 KB until the live effects, 112 KB until the start-up show (see the
# docstring).
WASM_GZ_BUDGET = 116 * 1024
# 48 KB until the Library's redesign (the Details panel, the category tabs,
# Make art as a view): the UI is served from the flash pack at ~320 KB/s, so
# +8 KB is ~25 ms of a page load. 56 KB until display names, the platform
# chips and Now's empty-playlist line (2026-10-21).
JS_CSS_GZ_BUDGET = 57 * 1024
ART_WASM_GZ_BUDGET = 150 * 1024
STUDIO_GZ_BUDGET = 24 * 1024
STUDIO = {"js/studio.js", "studio.css"}
LIVE_GZ_BUDGET = 4 * 1024
LIVE = {"js/live.js"}
BOOT_GZ_BUDGET = 2 * 1024
BOOT = {"js/boot.js"}
POWER_GZ_BUDGET = 2 * 1024
POWER = {"js/power.js"}
DETAILS_GZ_BUDGET = 8 * 1024
DETAILS = {"js/details.js"}
FIND_GZ_BUDGET = 10 * 1024
FIND = {"js/find.js", "find.css"}
SMOOTH_GZ_BUDGET = 6 * 1024
SMOOTH = {"js/smooth.js"}
CARD_GZ_BUDGET = 2 * 1024
CARD = {"js/card.js"}
FIRMWARE_GZ_BUDGET = 4 * 1024
FIRMWARE = {"js/firmware.js"}
UPDATE_GZ_BUDGET = 4 * 1024
UPDATE = {"js/update.js"}
PANEL_GZ_BUDGET = 2 * 1024
PANEL = {"js/panel.js"}
PANEL_TEST_GZ_BUDGET = 2 * 1024
PANEL_TEST = {"js/paneltest.js"}
API_GZ_BUDGET = 14 * 1024
API = {"api.html"}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("root", nargs="?", default="opendmd-web/www")
    ap.add_argument("--pack", help="the www pack built from it")
    ap.add_argument("--pack-limit", type=int, default=1_310_720, help="the www partition, bytes")
    args = ap.parse_args()
    root = Path(args.root)
    files = sorted(p for p in root.rglob("*") if p.is_file() and p.suffix != ".gz"
                   and not p.name.startswith("."))
    totals: dict[str, list[int]] = {}
    print(f"{'file':<40} {'raw':>9} {'gz':>9}")
    for p in files:
        raw = p.stat().st_size
        gz_path = p.with_name(p.name + ".gz")
        gz = gz_path.stat().st_size if gz_path.exists() else raw
        rel = p.relative_to(root).as_posix()
        if rel.startswith("pkg/"):
            kind = "wasm+glue"
        elif rel.startswith("pkg-art/"):
            kind = "art wasm"
        elif rel in STUDIO:
            kind = "studio"
        elif rel in LIVE:
            kind = "live"
        elif rel in BOOT:
            kind = "boot"
        elif rel in POWER:
            kind = "power"
        elif rel in DETAILS:
            kind = "details"
        elif rel in FIND:
            kind = "find"
        elif rel in SMOOTH:
            kind = "smooth"
        elif rel in FIRMWARE:
            kind = "firmware"
        elif rel in UPDATE:
            kind = "update"
        elif rel in PANEL:
            kind = "panel"
        elif rel in PANEL_TEST:
            kind = "panel test"
        elif rel in CARD:
            kind = "card"
        elif rel in API:
            kind = "api page"
        elif rel.startswith("art/"):
            kind = "art index"
            t = totals.setdefault(kind, [0, 0])
            t[0] += raw
            t[1] += gz
            continue
        else:
            kind = {".js": "js+css", ".css": "js+css"}.get(p.suffix, "other")
        t = totals.setdefault(kind, [0, 0])
        t[0] += raw
        t[1] += gz
        print(f"{str(p.relative_to(root)):<40} {raw:>9,} {gz:>9,}")
    print()
    ok = True
    for kind, budget in (("wasm+glue", WASM_GZ_BUDGET), ("js+css", JS_CSS_GZ_BUDGET),
                         ("art wasm", ART_WASM_GZ_BUDGET), ("studio", STUDIO_GZ_BUDGET),
                         ("live", LIVE_GZ_BUDGET), ("boot", BOOT_GZ_BUDGET),
                         ("power", POWER_GZ_BUDGET), ("details", DETAILS_GZ_BUDGET), ("find", FIND_GZ_BUDGET),
                         ("smooth", SMOOTH_GZ_BUDGET), ("firmware", FIRMWARE_GZ_BUDGET), ("update", UPDATE_GZ_BUDGET), ("card", CARD_GZ_BUDGET),
                         ("panel", PANEL_GZ_BUDGET), ("panel test", PANEL_TEST_GZ_BUDGET),
                         ("api page", API_GZ_BUDGET),
                         ("art index", None), ("other", None)):
        raw, gz = totals.get(kind, [0, 0])
        verdict = "" if budget is None else ("ok" if gz <= budget else "OVER BUDGET")
        limit = "" if budget is None else f"/ {budget // 1024} KB"
        print(f"{kind:<10} raw {raw:>9,}           gz {gz:>9,} {limit:<10} {verdict}")
        ok &= budget is None or gz <= budget
    all_raw = sum(t[0] for t in totals.values())
    all_gz = sum(t[1] for t in totals.values())
    print(f"{'total':<10} raw {all_raw:>9,}           gz {all_gz:>9,}")
    if args.pack:
        n = Path(args.pack).stat().st_size
        print(f"pack: {n:,} B of {args.pack_limit // 1024:,} KB ({n * 100 / args.pack_limit:.1f}%)")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
