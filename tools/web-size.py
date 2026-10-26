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
hand-written JS + CSS <= 60 KB (40 KB until Follow RePlayOS's Settings card,
which had ~0.6 KB left; 48 KB until the Library's redesign — the Details
panel, the category tabs, Make art as a view —, +8 KB being ~25 ms of a
page load from flash; 56 KB until display names and the platform chips,
2026-10-21, after the panel test and web-ux merges; 57 KB until the UX
refresh, 2026-10-22, which treats 60 KB as a ceiling). The Studio ("Make art") is loaded only when it
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
imported when first opened), <= 2 KB, and so does Settings' Device tab
(`js/device.js`: the status, the playback cache, network and the live
rename, reload and reboot, About; imported when the tab is first open),
<= 7.5 KB (7 KB until enterprise WiFi networks were greyed out in its scan
list, 2026-10-26), and so does the
headings' pixel font (`fonts/silkscreen.woff2`, Silkscreen subset to ASCII
and a few marks by tools/font-subset.py; preloaded by index.html, so it is
part of every page load, but it is not JS or CSS), <= 4 KB; its licence
(`fonts/OFL.txt`, packed beside it, never fetched by the page) is "other".
The Library's Formulas (the editor, the examples and the gallery:
`js/formulas.js`, `formulas.css` and the examples, `formulas.json`;
imported when the Formulas tab is first chosen or a `#formula=` link
opens) have a line of their own, <= 21 KB (16 until Psi phase 2, 17 until
layers and drawing over a GIF, 20 until parts), and so does their wasm module
(`pkg-psi/`: Psi, the language formulas are written in — its compiler and
VM, the firmware's own crate — built without std; loaded with them),
<= 43 KB (36 until layers and drawing over a GIF, 40 until parts — one
formula inside another, drawn at other places: the compiler's scopes,
re-reading and messages, ~+3 KB): the spike's 24.7 KB measured a module that never read an error
message, so the error texts and their formatting were dropped; with them,
the wasm-bindgen glue and std it was 40 KB, without std 34.7 KB.
The formula page on the site (`--share opendmd-web/share`: `f/`'s page,
script, style and QR encoder, gzipped here since `make web` does not
compress them; never packed) has a line of its own, <= 12 KB (the in-house
QR encoder is ~2.9 KB of it); it runs on the
demo's files (the main wasm, js/preview.js and its imports, js/psikit.js,
pkg-psi), which are counted in their own lines and printed as the page's
first-visit total for information. js/psikit.js (the share link's encoder
and decoder, the highlighting) counts with the Formulas line. Their "Show
on panel" (`js/psishow.js`: the formula's path, the throttled live edits
and their errors; imported with the view on a device only) has a line of
its own, <= 2 KB. Any Formula as the panel's look (`js/looks.js`: the
blend and amount controls, the defaults, the panel's refusals and the save;
imported by Settings' Look tab and Formulas' "Use as the look"; 2026-10-26,
branch any-look) has a line of its own, <= 3 KB, so neither the 60 KB page
line nor the Formulas line carries it.
The API reference is not packed (until 2026-10-23 it was `api.html`, its
own line, <= 15 KB, ~14.5 KB gz): Settings -> About links the HTTP API on
the project site, docs/api.md as each release published it.
The web demo's player (`js/demo.js`: the device's settings API answered in
the page for the site's demo, and what the settings do to its panel;
imported by app.js only when the page is the demo; `opendmd-pack www`
leaves it out of the pack, so a player never carries it) has a line of its
own, <= 8 KB.
The Library's folder tree and the row thumbnails (`js/tree.js`,
`js/thumbs.js` and `tree.css`, which holds the playlist editor's panes too;
imported with the Library or the editor, never by the first page) have a
line of their own, <= 13 KB (12 until the review's fixes), and so does the playlist editor
(`js/pledit.js`, imported the first time a playlist opens), <= 6 KB; both
left the js+css line (2026-10-26: 59,646 -> 53,659 B gz).
The third-party licence notices (`licenses.txt`, the firmware's and the web
app's from licenses/, opened from Settings -> Device -> About in a tab of
its own, never part of a page load) have a line of their own, <= 40 KB:
they cost flash, not load time, and grow with the dependencies.
The name index for "Find art
for a game" is not packed (until 2026-10-26 it was `art/*.json`, ~527 KB
gz, half the pack): the site publishes it (tools/site.py, `art-index/`)
and find.js fetches it from there. Raw sizes are printed for
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
# chips and Now's empty-playlist line (2026-10-21); 57 KB until the UX
# refresh (Settings in tabs, the Playlists editor as a screen, 2026-10-22):
# a hard ceiling — the next feature pays for itself.
JS_CSS_GZ_BUDGET = 60 * 1024
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
DEVICE_GZ_BUDGET = 7 * 1024 + 512  # +512 2026-10-26: enterprise networks shown as not joinable
DEVICE = {"js/device.js"}
FONT_GZ_BUDGET = 4 * 1024
FONT = {"fonts/silkscreen.woff2"}
# The 40 KB and 20 KB below are Psi stage A's numbers (layers, drawing over
# a GIF; branch psi-layers, 2026-10-24), approved by the owner the same day.
# 43 KB for parts (2026-10-25): approved by the owner 2026-10-25. Built at
# opt-level "z" since then (42.6 KB; "s" was 44.2): the browser preview's
# frame is ~0.07 ms mean, 0.14 ms worst under node (0.06 / 0.10 at "s").
# 44 KB for the text built-ins (2026-10-26, branch psi-glyph: `digit`,
# `glyph`, `bit`, 'A' character codes): the panel's two fonts (1,235 B, ~570
# B gz) and their steps, +1.5 KB gz; a small raise the owner pre-approved.
# 45 KB for any Formula as a look (2026-10-26, branch any-look): `look()`,
# the look's text composed in the module from the compiled formula
# (opendmd_psi::look: one implementation, tested with golden frames, not a
# second copy in JS), +1.4 KB gz; a small raise under the owner's standing
# OK for budget lines while the firmware stays comfortable.
PSI_WASM_GZ_BUDGET = 45 * 1024  # 36 KB until 2026-10-24 (Psi layers and drawing over a GIF: +4 KB); 40 until parts (one formula inside another); 43 until the text built-ins; 44 until any-look
# 21 KB for parts (2026-10-25: `use … as p` pasting with its guards, three
# examples): approved by the owner 2026-10-25. 22 KB for Psi ergonomics
# stage 2 (2026-10-26, branch psi-ergo: the cheat sheet's lines for pick,
# colours and the values about the middle, the editor's colour swatches;
# +192 B gz, the rewritten examples 292 B shorter but only 22 B gz): a small
# raise, approved (the owner lets budget lines move with the firmware still comfortable).
FORMULAS_GZ_BUDGET = 22 * 1024  # 16 KB until 2026-10-23 (Psi phase 2: Show on panel), 17 until 2026-10-24 (over a GIF, 8 examples), 20 until parts, 21 until psi-ergo
FORMULAS = {"js/formulas.js", "js/psikit.js", "formulas.css", "formulas.json"}
SHOW_GZ_BUDGET = 2 * 1024
SHOW = {"js/psishow.js"}
# Any Formula as a look (branch any-look, 2026-10-26): a new line, ~2.0 KB at the time.
LOOKS_GZ_BUDGET = 3 * 1024
LOOKS = {"js/looks.js"}
SHARE_GZ_BUDGET = 12 * 1024
# The site's "Set up a card" page (share/cardprep, published at /card/; its
# tests/ aside): its own line since 2026-10-25, 16.9 KB at the time.
CARD_PAGE_GZ_BUDGET = 18 * 1024
# What the formula page loads from the demo (its own files aside).
SHARE_USES = ("pkg/", "pkg-psi/", "js/psikit.js", "js/preview.js", "js/state.js", "js/util.js", "fonts/silkscreen.woff2")
LICENCES_GZ_BUDGET = 40 * 1024
LICENCES = {"licenses.txt"}
# The web demo's player (js/demo.js: the settings API answered in the page,
# the schedule, what settings do to the demo's panel): imported by app.js
# only for the site's demo and left out of the pack (opendmd-pack www.rs
# DEMO_ONLY), so outside the 60 KB line.
DEMO_GZ_BUDGET = 8 * 1024
DEMO = {"js/demo.js"}
# The Library's folder tree and the thumbnails (2026-10-26, branch
# library-playlists): imported with the Library (and the playlist editor),
# never by the first page; the tree's and the editor's stylesheet with them.
# 13 KB since the review (2026-10-26): the failures that pass kept apart
# from those that do not, Retry-After, a stalled body's timeout, a folder's
# Try again row, ARIA positions; ~1 KB gz, the first page unchanged.
TREE_GZ_BUDGET = 13 * 1024
TREE = {"js/tree.js", "js/thumbs.js", "tree.css"}
# The playlist editor (#sets/edit), imported the first time a playlist opens.
EDITOR_GZ_BUDGET = 6 * 1024
EDITOR = {"js/pledit.js"}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("root", nargs="?", default="opendmd-web/www")
    ap.add_argument("--pack", help="the www pack built from it")
    ap.add_argument("--pack-limit", type=int, default=1_310_720, help="the www partition, bytes")
    ap.add_argument("--share", help="the site's formula page sources (opendmd-web/share)")
    args = ap.parse_args()
    root = Path(args.root)
    files = sorted(p for p in root.rglob("*") if p.is_file() and p.suffix != ".gz"
                   and not p.name.startswith("."))
    stray = [p for p in files if p.relative_to(root).parts[0] == "art"]
    if stray:
        print(f"{root}/art/ is the art index, which the site publishes (opendmd-web/art-index/, "
              f"tools/art-index.py --out): it must not be packed ({len(stray)} files)")
        return 1
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
        elif rel.startswith("pkg-psi/"):
            kind = "psi wasm"
        elif rel in FORMULAS:
            kind = "formulas"
        elif rel in SHOW:
            kind = "show"
        elif rel in LOOKS:
            kind = "looks"
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
        elif rel in DEVICE:
            kind = "device"
        elif rel in FONT:
            kind = "font"
        elif rel in LICENCES:
            kind = "licences"
        elif rel in DEMO:
            kind = "demo"
        elif rel in TREE:
            kind = "tree"
        elif rel in EDITOR:
            kind = "editor"
        else:
            kind = {".js": "js+css", ".css": "js+css"}.get(p.suffix, "other")
        t = totals.setdefault(kind, [0, 0])
        t[0] += raw
        t[1] += gz
        print(f"{str(p.relative_to(root)):<40} {raw:>9,} {gz:>9,}")
    print()
    ok = True
    if args.share:
        import gzip
        share = Path(args.share)
        sraw = sgz = craw = cgz = 0
        for p in sorted(q for q in share.rglob("*") if q.is_file() and "tests" not in q.relative_to(share).parts):
            b = p.read_bytes()
            g = len(gzip.compress(b, 9, mtime=0))
            if p.relative_to(share).parts[0] == "cardprep":
                craw += len(b)
                cgz += g
            else:
                sraw += len(b)
                sgz += g
            print(f"{'share/' + p.relative_to(share).as_posix():<40} {len(b):>9,} {g:>9,}")
        uses = sum((q.with_name(q.name + ".gz").stat().st_size if q.with_name(q.name + ".gz").exists() else q.stat().st_size)
                   for q in files if any(q.relative_to(root).as_posix().startswith(u) for u in SHARE_USES))
        verdict = "ok" if sgz <= SHARE_GZ_BUDGET else "OVER BUDGET"
        print(f"{'share':<10} raw {sraw:>9,}           gz {sgz:>9,} / {SHARE_GZ_BUDGET // 1024} KB {verdict:<10}"
              f"(first visit with the demo's files: {sgz + uses:,} B gz)")
        cverdict = "ok" if cgz <= CARD_PAGE_GZ_BUDGET else "OVER BUDGET"
        print(f"{'card page':<10} raw {craw:>9,}           gz {cgz:>9,} / {CARD_PAGE_GZ_BUDGET // 1024} KB {cverdict}")
        print()
        ok &= sgz <= SHARE_GZ_BUDGET and cgz <= CARD_PAGE_GZ_BUDGET
    for kind, budget in (("wasm+glue", WASM_GZ_BUDGET), ("js+css", JS_CSS_GZ_BUDGET),
                         ("art wasm", ART_WASM_GZ_BUDGET), ("studio", STUDIO_GZ_BUDGET),
                         ("live", LIVE_GZ_BUDGET), ("boot", BOOT_GZ_BUDGET),
                         ("power", POWER_GZ_BUDGET), ("details", DETAILS_GZ_BUDGET), ("find", FIND_GZ_BUDGET),
                         ("smooth", SMOOTH_GZ_BUDGET), ("firmware", FIRMWARE_GZ_BUDGET), ("update", UPDATE_GZ_BUDGET), ("card", CARD_GZ_BUDGET),
                         ("panel", PANEL_GZ_BUDGET), ("panel test", PANEL_TEST_GZ_BUDGET), ("device", DEVICE_GZ_BUDGET),
                         ("psi wasm", PSI_WASM_GZ_BUDGET), ("formulas", FORMULAS_GZ_BUDGET), ("show", SHOW_GZ_BUDGET), ("looks", LOOKS_GZ_BUDGET),
                         ("font", FONT_GZ_BUDGET), ("licences", LICENCES_GZ_BUDGET), ("demo", DEMO_GZ_BUDGET),
                         ("tree", TREE_GZ_BUDGET), ("editor", EDITOR_GZ_BUDGET),
                         ("other", None)):
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
