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
HUB75 timing selects, the trial's Keep/Undo and Reset to defaults, and the
Moving pictures card: panel_sync and panel_split; imported with Settings ›
Device, with js/firmware.js), <= 3 KB (2 KB until Moving pictures, 2,353 B
gz with it), and
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
`js/formulas.js`, `formulas.css` and the bundled ones, `formulas.json`,
which make web writes from opendmd-web/formulas/'s `-- bundle` files;
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
PANEL_GZ_BUDGET = 3 * 1024
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
# 52 KB for the draw stage (2026-10-27, branch psi-draw, docs/psi-drawing-design.md
# phase 1: `draw { }`, `for`, `table`, `line`, `dot`, `ink`): 45,589 ->
# 51,892 B gz, +6.3 KB (the design guessed +3-4): the parser's new
# statements and their sentences (~2.5 KB raw of text), the emitter's fourth
# section, the draw-stage interpreter and its integer rasteriser, the cost
# model's draw part; +2 KB raw of zero `.data` (two Programs in the module's
# static state, each 1 KB of tables bigger) that gzips to almost nothing.
# Loaded only when Formulas opens; raised under the owner's standing OK for
# budget lines while the firmware stays comfortable (it does: see
# docs/research/size-budget.md).
# 58 KB for Psi phase 2 (2026-10-27, branch psi-p2, docs/psi-drawing-design.md
# "As built (phase 2)"): the still cache (the compiler choosing position-only
# values and laying out their run, the VM keeping and copying them, the
# fingerprint), the column run, `pal(P, v) * k` and noise at a whole z, and
# the estimate with the cache: +5.4 KB gz of the module's wasm (49.2 -> 54.6
# KB measured alone; a ~3 KB stable sort and two monomorphised loops were
# taken out to get there). What lets Tunnel and Rotozoom be Formulas at
# 50 fps; raised under the owner's standing OK for budget lines.
# 62 KB for psi-speed (2026-10-27, docs/research/psi-speed.md): the ranges
# pass (`range`: every value's interval, the narrowed steps chosen at
# compile time, so the editor's estimate is the panel's) and the
# palette-scale reuse: +2.8 KB gz (59.1 -> 61.9 KB measured with today's
# toolchain; the narrowed steps' own loops are left out of wasm, which runs
# their checked forms: the same bytes). What takes Tunnel towards 100 fps on
# the panel; raised under the owner's standing OK for budget lines.
# 67 KB for `loop forever` (2026-10-27, branch psi-lang, docs/research/
# endless-road.md §5.1): Formulas that go on without repeating — the
# compiler's rewriting of a value that grows without end into once-a-frame
# wide steps and the exact ways back (`Dag::wide_op`, ~6 KB of wasm before
# wasm-opt), the wide steps themselves, the wide noise and its lane loop,
# `seed`: +4.6 KB gz (61.9 -> ~66.5 KB with its glue). Raised under the
# owner's OK to move the psi wasm line (this branch's brief).
# 71 KB for sprites (2026-10-28, branch psi-lang, endless-road.md §5.2):
# `sprite NAME { … }` blocks — the lexer's raw rows, the key and rows parsed
# and packed (bytes, not `str`'s Unicode trims), their nine error sentences,
# the sprite step's lane loop: +3.6 KB gz (~67.9 -> ~71.5 KB with its glue).
# Raised under the brief's OK.
# 73 KB for slot reuse (2026-10-28, branch psi-lang, endless-road.md §5.3):
# the compiler lays steps out on virtual slots and gives each a scalar slot
# by a linear scan of live intervals (`assign_slots`, ~3.5 KB of wasm):
# +1.7 KB gz (~71.5 -> ~73.2 KB). Instead of 256 slots, which would have
# grown every formula's memory on the device. Raised under the brief's OK.
# 74 KB for palettes of a formula's own (2026-10-28, branch psi-lang,
# endless-road.md §5.4): `palette NAME = #…, …`, its spread over 32
# entries, its sentences and the unpacking in the colour step: +1.5 KB gz
# (~73.2 -> ~74.7 KB). Raised under the brief's OK.
# 76 KB for psi-speed2 (2026-10-28, docs/research/psi-speed2.md): the
# compiler's fused runs (`fuse.rs`: a checker's floors, add and mask, a
# multiply-add, a palette colour of a choice or a pick, a colour of the
# canvas alone, each a
# marker before the steps) and the draw loops' lane analysis
# (`Program::vector_loop`), both in the download only so the editor's
# estimate stays the panel's — the browser runs neither (the steps and a
# turn at a time draw the same bytes): +2.2 KB gz (~74.9 -> ~77.2 KB).
# Raised under the brief's OK (it may move a budget with a reason).
# 82 KB for formula-apps F1 (2026-10-28, branch formula-f1,
# docs/research/formula-apps.md): the declarations (`psi 1`, `name`,
# `about`, `control`, `look`: the header reader, text in quotes, their
# fifteen sentences), `Meta` and the declared-look composition, and the
# `controls()` export (the JSON the page builds its sliders and buttons
# from, so the browser never parses a declaration itself): ~77.2 ->
# 83,063 B gz (+3.9 KB; formula-apps.md §4 guessed 1-1.5). Raised under the
# brief's OK (it may move the psi wasm line with a reason).
# 83 KB after merging main's psi-speed3 follow-ups into formula-f1 (the
# review's fixes: one finishing pass, codes, E969's character check):
# 83,842 B gz, 126 B under 82 KB; raised for room, same OK.
# 84 KB for formula-apps F2 (2026-10-28, branch formula-f2): the Formula
# settings sheet's text edits (`declare`: a declaration line set, put in
# among the others or taken out; `look` now that edit of the `look` line)
# and `asLook` (the look the panel draws for a formula's own line, for
# Settings' and the sheet's checks and the demo): 83,842 -> 85,225 B gz
# (+1.4 KB; the edit is bytes and `get` only — a str slice's panic alone
# was +5 KB of core::fmt). Raised under the brief's OK (budgets may move
# with written reasons).
PSI_WASM_GZ_BUDGET = 84 * 1024  # 83 until formula-apps F2; 82 until main's psi-speed3 merge; 76 until formula-apps F1; 74 until psi-speed2; 73 until palettes; 71 until slot reuse; 67 until sprites; 62 until loop forever; 58 until psi-speed; 36 KB until 2026-10-24 (Psi layers and drawing over a GIF: +4 KB); 40 until parts (one formula inside another); 43 until the text built-ins; 44 until any-look; 45 until the draw stage; 52 until phase 2
# 21 KB for parts (2026-10-25: `use … as p` pasting with its guards, three
# examples): approved by the owner 2026-10-25. 22 KB for Psi ergonomics
# stage 2 (2026-10-26, branch psi-ergo: the cheat sheet's lines for pick,
# colours and the values about the middle, the editor's colour swatches;
# +192 B gz, the rewritten examples 292 B shorter but only 22 B gz): a small
# raise, approved (the owner lets budget lines move with the firmware still comfortable).
# 24 KB for the draw stage (2026-10-27, branch psi-draw): the cheat sheet's
# Drawing section, the highlighter's words, "Draws N lines and dots a frame",
# and two drawing examples (Starfield, now drawn, and Wireframe cube):
# 22,076 -> ~23.1 KB gz; raised under the same standing OK.
# 25 KB for pickers and Add to playlist (2026-10-27, branch psi-p2-web): a
# slider with choices (`-- a: shape = cube, ball`) as a row of buttons, in
# js/psikit.js (shared with the site's /f/ page), and Add to playlist on every
# formula (what lets the Library's Effects tab go): 23,614 -> ~24.8 KB gz.
# Raised under the same standing OK.
# 27 KB for Psi phase 2 (2026-10-27, branch psi-p2): two more examples
# (Tunnel, Rotozoom: the built-in effects' looks, ~1 KB gz of formulas.json),
# the estimate with the still cache in the ratings, the cheat sheet's
# `pal(fire, v) * k`: 24,811 -> ~26.0 KB gz. Same standing OK.
# 28 KB for `loop forever` (2026-10-27, branch psi-lang): its cheat-sheet
# row, the highlighter's `forever`, "Goes on without repeating", Save as
# GIF's first 10 s: 27,612 -> ~27.8 KB gz with Endless road rewritten for it.
# Raised under the brief's OK.
# 30 KB for the Time gate example (2026-10-28, branch ct-gate): one more
# example in formulas.json (the CT time gate, 19 lines of code and a dozen
# of comments): 28,435 -> 29,419 B gz. Time gate example; the Formulas split
# (docs/research/formula-apps.md, pending) will move most examples out of
# the firmware bundle.
# 31 KB for formula-apps F1 (2026-10-28, branch formula-f1): psikit.js
# builds every control from pkg-psi's controls() (control, fit, num) and
# now holds the path's compact() that psishow's fpath sends (moved from
# psishow, made to match opendmd_psi::compact exactly: quotes on declaration
# lines, '"' as a character; a psi-web test runs it under node), the
# cheat sheet's declaration rows: 29,419 -> 30,985 B gz. formula-apps F3
# (most examples out of the firmware's formulas.json) gives ~2.5 KB back.
# 32 KB for formula-apps F2 (2026-10-28, branch formula-f2): the path's
# encoder moved here from psishow.js (psikit `path`, now with the values at
# their declared defaults left out and `about` dropped over 2,048 B, held
# to the compiler by psi-web's node test), the editor's name as the `name`
# line and its Settings button, tiles with `about` and a look badge:
# 30,985 -> 32,488 B gz (the sheet itself is its own line, js/fsheet.js).
# Under the brief's OK; F3 (most examples out of formulas.json) gives
# ~2.5 KB back and should lower it again.
# 27 KB for formula-apps F3 (2026-10-28, branch formula-f3): the split.
# formulas.json is written by make web from the ten `-- bundle` files in
# opendmd-web/formulas/ (the rest are the site gallery's), as `[{src}]`
# alone (name and about are the sources' own lines): 21,179 B raw / 8,270
# gz -> 4,685 / 2,094; the view's two example cards became one Bundled card
# and a More Formulas link: 32,488 -> 26,584 B gz. Lowered back so the room
# isn't spent by accident (1 KB of headroom, not the old 32).
# 30 KB for the revised bundle (owner, 2026-10-28): Endless road and Time
# gate in, Clock and Corner clock out; the two are the largest examples
# (26,584 -> 28,806 B gz, all of it formulas.json).
FORMULAS_GZ_BUDGET = 30 * 1024  # 27 until Endless road and Time gate were bundled; 32 until formula-apps F3; 31 until formula-apps F2; 30 until formula-apps F1; 28 until Time gate; 27 until loop forever; 16 KB until 2026-10-23 (Psi phase 2: Show on panel), 17 until 2026-10-24 (over a GIF, 8 examples), 20 until parts, 21 until psi-ergo, 22 until the draw stage, 24 until pickers, 25 until phase 2
FORMULAS = {"js/formulas.js", "js/psikit.js", "formulas.css", "formulas.json"}
SHOW_GZ_BUDGET = 2 * 1024
SHOW = {"js/psishow.js"}
# Any Formula as a look (branch any-look, 2026-10-26): a new line, ~2.0 KB at the time.
LOOKS_GZ_BUDGET = 3 * 1024
LOOKS = {"js/looks.js"}
# The Formula settings sheet (formula-apps F2, 2026-10-28, branch
# formula-f2): name, about, controls and look as edits of the Formula's
# declaration lines; imported when the editor's Settings first opens, so a
# line of its own rather than the Formulas line (which every visit to the
# view loads).
FSHEET_GZ_BUDGET = 3 * 1024
FSHEET = {"js/fsheet.js"}
SHARE_GZ_BUDGET = 12 * 1024
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
        elif rel in FSHEET:
            kind = "fsheet"
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
        sraw = sgz = 0
        for p in sorted(q for q in share.rglob("*") if q.is_file() and "tests" not in q.relative_to(share).parts):
            b = p.read_bytes()
            g = len(gzip.compress(b, 9, mtime=0))
            sraw += len(b)
            sgz += g
            print(f"{'share/' + p.relative_to(share).as_posix():<40} {len(b):>9,} {g:>9,}")
        uses = sum((q.with_name(q.name + ".gz").stat().st_size if q.with_name(q.name + ".gz").exists() else q.stat().st_size)
                   for q in files if any(q.relative_to(root).as_posix().startswith(u) for u in SHARE_USES))
        verdict = "ok" if sgz <= SHARE_GZ_BUDGET else "OVER BUDGET"
        print(f"{'share':<10} raw {sraw:>9,}           gz {sgz:>9,} / {SHARE_GZ_BUDGET // 1024} KB {verdict:<10}"
              f"(first visit with the demo's files: {sgz + uses:,} B gz)")
        print()
        ok &= sgz <= SHARE_GZ_BUDGET
    for kind, budget in (("wasm+glue", WASM_GZ_BUDGET), ("js+css", JS_CSS_GZ_BUDGET),
                         ("art wasm", ART_WASM_GZ_BUDGET), ("studio", STUDIO_GZ_BUDGET),
                         ("live", LIVE_GZ_BUDGET), ("boot", BOOT_GZ_BUDGET),
                         ("power", POWER_GZ_BUDGET), ("details", DETAILS_GZ_BUDGET), ("find", FIND_GZ_BUDGET),
                         ("smooth", SMOOTH_GZ_BUDGET), ("firmware", FIRMWARE_GZ_BUDGET), ("update", UPDATE_GZ_BUDGET), ("card", CARD_GZ_BUDGET),
                         ("panel", PANEL_GZ_BUDGET), ("panel test", PANEL_TEST_GZ_BUDGET), ("device", DEVICE_GZ_BUDGET),
                         ("psi wasm", PSI_WASM_GZ_BUDGET), ("formulas", FORMULAS_GZ_BUDGET), ("show", SHOW_GZ_BUDGET), ("looks", LOOKS_GZ_BUDGET),
                         ("fsheet", FSHEET_GZ_BUDGET),
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
