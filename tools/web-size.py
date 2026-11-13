# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Print the web app's size, raw and gzipped, and enforce the budget.

    uv run tools/web-size.py opendmd-web/www-dist [--pack opendmd-web/www.pack --pack-limit 786432 --pack-spare 65536] [--unminified]

It measures what ships: `make web`'s output directory (tools/web-minify.py:
the sources with every script and stylesheet, wasm-pack's glue included,
minified by esbuild, then gzipped), never the commented sources in
opendmd-web/www. Every line was re-based on that output on 2026-11-05
(branch web-minify) with its ceiling kept: the main JS + CSS line went from
61,413 to 39,549 B gz of its 60 KB, and the rule is unchanged — the next
feature pays for itself (opendmd-notes docs/research/size-budget.md "Minified build").
`--unminified` (`make web WEB_MINIFY=0`, a debugging build) prints the
same table but enforces no line: the ceilings are for minified files.

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
above. Settings' Start-up card (`js/boot.js`, imported by the Settings view: the
`boot_show` select, the start-up Formula's picker, `boot_formula`, and the previews of the start-up show) has a line of its own
too, <= 3 KB (2 KB until the start-up Formula, 2026-11-09: its picker, its
preview through a pkg-psi instance of its own and the panel's word when it
fell back took the line from 756 to ~2,000 B gz), and so does Settings' Power card (`js/power.js`: the Maximum brightness
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
so does the first-run banner offering an API key (`js/keynag.js`, imported
only while the panel has none), <= 1 KB, and
so does Now's card format dialog (`js/card.js`: "Format
card…" for a card the panel can't read, the POST and the wait for it;
imported when first opened), <= 2 KB, and so does Settings' Device tab
(`js/device.js`: the status, the playback cache, network and the live
rename, reload and reboot, About; imported when the tab is first open),
<= 7.5 KB (7 KB until enterprise WiFi networks were greyed out in its scan
list, 2026-10-26; the work or school login that replaced that, 2026-11-06,
fits it), and so does the
headings' pixel font (`fonts/silkscreen.woff2`, Silkscreen subset to ASCII
and a few marks by tools/font-subset.py; preloaded by index.html, so it is
part of every page load, but it is not JS or CSS), <= 4 KB; its licence
(`fonts/OFL.txt`, packed beside it, never fetched by the page) is "other".
The Library's Formulas (the editor, the examples and the gallery:
`js/formulas.js`, `formulas.css` and the bundled ones, `formulas.json`,
which make web writes from opendmd-web/formulas/'s `-- bundle` files;
imported when the Formulas tab is first chosen or a `#gallery=`/`#remix` link
opens) have a line of their own, <= 21 KB (16 until Psi phase 2, 17 until
layers and drawing over a GIF, 20 until parts), and so does their wasm module
(`pkg-psi/`: Psi, the language formulas are written in — its compiler and
VM, the firmware's own crate — built without std; loaded with them),
<= 43 KB (36 until layers and drawing over a GIF, 40 until parts — one
formula inside another, drawn at other places: the compiler's scopes,
re-reading and messages, ~+3 KB): the spike's 24.7 KB measured a module that never read an error
message, so the error texts and their formatting were dropped; with them,
the wasm-bindgen glue and std it was 40 KB, without std 34.7 KB.
The Formula page on the site (`--share opendmd-web/share`: `f/`'s page,
script and style, gzipped here since `make web` does not compress them;
never packed) has a line of its own, <= 8 KB (Create Art's page, `create/`,
another, <= 5 KB, on the Studio's lines; the Studio's own page, `studio/`,
the site's pixel editor, another, <= 32 KB, and what it loads later, <= 5 KB); it is the web app's editor on
the demo's files (the main wasm, js/formulas.js and its imports, pkg-psi,
app.css and formulas.css), which are counted in their own lines and printed
as the page's first-visit total for information. js/psikit.js (where a
link says a Formula comes from, the highlighting) counts with the Formulas
line; Submit to the gallery (`js/fsubmit.js`, the editor's and the page's
form) has a line of its own, <= 4 KB. Their "Show
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
The Formulas editor's names at the caret and its cheat sheet
(`js/fhint.js`, imported at the editor's first key) have a line of their
own, <= 7 KB.
The Formulas editor's error codes (`js/psierr.js`, imported at a formula's
first error) have a line of their own, <= 4 KB, and so does the site's Psi
language reference script (`--share`'s `ref/psiref.js` with the drawing it
shares, `ref/psiled.js`, outside the share line), <= 6 KB; the home page's
live panels (`ref/home.js`) and the gallery's tiles on hover
(`ref/ftiles.js`) a line each, with psiled.js, <= 4 KB; the hero's
show (`ref/hero.js`: the panel's transitions, a GIF decoder and the
playlist; imported by home.js after the page has loaded) a line of its
own, <= 5 KB, and so do the web app's LEDs over the home page's
screenshots (`ref/leds.js`), <= 2 KB; the gallery's
sorting and filtering (`ref/ftsort.js`) a line of its own, <= 3 KB.
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
flash), its size signed (the pack and its 72-byte signature trailer) against
the partition (768 KB in table v4) and what that leaves — "pack: N B
signed of 768 KB (P%), S B spare (need 65536)". The budgets above are
about load time over WiFi, not flash. The partition itself is enforced by
the packer (`opendmd-pack www --max-bytes`); this line fails the build when
the signed pack leaves less than `--pack-spare` (64 KB: the web app's
headroom, as the firmware keeps 64 KB of `factory`), so a pack that is
close to the partition is noticed before a release that cannot be
installed.
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
# exact() and pipeline::render_exact (the site's Studio's Import in full
# colour, studio-tidy 2026-11-12, ~1.7 KB gz) are site-only code living in
# the device's art wasm: the first candidate to gate behind a cargo feature
# (a site-only build of pkg-art) when this line gets tight.
ART_WASM_GZ_BUDGET = 150 * 1024
STUDIO_GZ_BUDGET = 24 * 1024
STUDIO = {"js/studio.js", "studio.css"}
BOOT_GZ_BUDGET = 3 * 1024
BOOT = {"js/boot.js"}
POWER_GZ_BUDGET = 2 * 1024
POWER = {"js/power.js"}
DETAILS_GZ_BUDGET = 8 * 1024
DETAILS = {"js/details.js"}
# A GIF from the site's Create Art (js/importgif.js: `#import`, the
# postMessage checks; loaded only then): a line of its own, <= 2 KB
# (2026-11-02, branch create-art: ~1.4 KB gz).
IMPORT_GZ_BUDGET = 2 * 1024
IMPORT = {"js/importgif.js"}
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
# The first-run nudge to set an API key (js/keynag.js, imported only while
# the panel has none; the security review's H3, 2026-11-05): <= 1 KB.
KEYNAG_GZ_BUDGET = 1024
KEYNAG = {"js/keynag.js"}
DEVICE_GZ_BUDGET = 7 * 1024 + 512  # +512 2026-10-26: enterprise networks shown as not joinable (now joinable: the login fields)
DEVICE = {"js/device.js"}
FONT_GZ_BUDGET = 4 * 1024
FONT = {"fonts/silkscreen.woff2"}
# The 40 KB and 20 KB below are Psi stage A's numbers (layers, drawing over
# a GIF; branch psi-layers, 2026-10-24).
# 43 KB for parts (2026-10-25). Built at
# opt-level "z" since then (42.6 KB; "s" was 44.2): the browser preview's
# frame is ~0.07 ms mean, 0.14 ms worst under node (0.06 / 0.10 at "s").
# 44 KB for the text built-ins (2026-10-26, branch psi-glyph: `digit`,
# `glyph`, `bit`, 'A' character codes): the panel's two fonts (1,235 B, ~570
# B gz) and their steps, +1.5 KB gz; a small raise.
# 45 KB for any Formula as a look (2026-10-26, branch any-look): `look()`,
# the look's text composed in the module from the compiled formula
# (opendmd_psi::look: one implementation, tested with golden frames, not a
# second copy in JS), +1.4 KB gz; a small raise, with the firmware still
# comfortable.
# 52 KB for the draw stage (2026-10-27, branch psi-draw, docs/psi-drawing-design.md
# phase 1: `draw { }`, `for`, `table`, `line`, `dot`, `ink`): 45,589 ->
# 51,892 B gz, +6.3 KB (the design guessed +3-4): the parser's new
# statements and their sentences (~2.5 KB raw of text), the emitter's fourth
# section, the draw-stage interpreter and its integer rasteriser, the cost
# model's draw part; +2 KB raw of zero `.data` (two Programs in the module's
# static state, each 1 KB of tables bigger) that gzips to almost nothing.
# Loaded only when Formulas opens; raised with the firmware still
# comfortable (see opendmd-notes docs/research/size-budget.md).
# 58 KB for Psi phase 2 (2026-10-27, branch psi-p2, docs/psi-drawing-design.md
# "As built (phase 2)"): the still cache (the compiler choosing position-only
# values and laying out their run, the VM keeping and copying them, the
# fingerprint), the column run, `pal(P, v) * k` and noise at a whole z, and
# the estimate with the cache: +5.4 KB gz of the module's wasm (49.2 -> 54.6
# KB measured alone; a ~3 KB stable sort and two monomorphised loops were
# taken out to get there). What lets Tunnel and Rotozoom be Formulas at
# 50 fps.
# 62 KB for psi-speed (2026-10-27, opendmd-notes docs/research/psi-speed.md): the ranges
# pass (`range`: every value's interval, the narrowed steps chosen at
# compile time, so the editor's estimate is the panel's) and the
# palette-scale reuse: +2.8 KB gz (59.1 -> 61.9 KB measured with today's
# toolchain; the narrowed steps' own loops are left out of wasm, which runs
# their checked forms: the same bytes). What takes Tunnel towards 100 fps on
# the panel.
# 67 KB for `loop forever` (2026-10-27, branch psi-lang, opendmd-notes docs/research/
# endless-road.md §5.1): Formulas that go on without repeating — the
# compiler's rewriting of a value that grows without end into once-a-frame
# wide steps and the exact ways back (`Dag::wide_op`, ~6 KB of wasm before
# wasm-opt), the wide steps themselves, the wide noise and its lane loop,
# `seed`: +4.6 KB gz (61.9 -> ~66.5 KB with its glue).
# 71 KB for sprites (2026-10-28, branch psi-lang, endless-road.md §5.2):
# `sprite NAME { … }` blocks — the lexer's raw rows, the key and rows parsed
# and packed (bytes, not `str`'s Unicode trims), their nine error sentences,
# the sprite step's lane loop: +3.6 KB gz (~67.9 -> ~71.5 KB with its glue).
# 73 KB for slot reuse (2026-10-28, branch psi-lang, endless-road.md §5.3):
# the compiler lays steps out on virtual slots and gives each a scalar slot
# by a linear scan of live intervals (`assign_slots`, ~3.5 KB of wasm):
# +1.7 KB gz (~71.5 -> ~73.2 KB). Instead of 256 slots, which would have
# grown every formula's memory on the device.
# 74 KB for palettes of a formula's own (2026-10-28, branch psi-lang,
# endless-road.md §5.4): `palette NAME = #…, …`, its spread over 32
# entries, its sentences and the unpacking in the colour step: +1.5 KB gz
# (~73.2 -> ~74.7 KB).
# 76 KB for psi-speed2 (2026-10-28, opendmd-notes docs/research/psi-speed2.md): the
# compiler's fused runs (`fuse.rs`: a checker's floors, add and mask, a
# multiply-add, a palette colour of a choice or a pick, a colour of the
# canvas alone, each a
# marker before the steps) and the draw loops' lane analysis
# (`Program::vector_loop`), both in the download only so the editor's
# estimate stays the panel's — the browser runs neither (the steps and a
# turn at a time draw the same bytes): +2.2 KB gz (~74.9 -> ~77.2 KB).
# 82 KB for formula-apps F1 (2026-10-28, branch formula-f1,
# opendmd-notes docs/research/formula-apps.md): the declarations (`psi 1`, `name`,
# `about`, `control`, `look`: the header reader, text in quotes, their
# fifteen sentences), `Meta` and the declared-look composition, and the
# `controls()` export (the JSON the page builds its sliders and buttons
# from, so the browser never parses a declaration itself): ~77.2 ->
# 83,063 B gz (+3.9 KB; formula-apps.md §4 guessed 1-1.5).
# 83 KB after merging main's psi-speed3 follow-ups into formula-f1 (the
# review's fixes: one finishing pass, codes, E969's character check):
# 83,842 B gz, 126 B under 82 KB; raised for room.
# 84 KB for formula-apps F2 (2026-10-28, branch formula-f2): the Formula
# settings sheet's text edits (`declare`: a declaration line set, put in
# among the others or taken out; `look` now that edit of the `look` line)
# and `asLook` (the look the panel draws for a formula's own line, for
# Settings' and the sheet's checks and the demo): 83,842 -> 85,225 B gz
# (+1.4 KB; the edit is bytes and `get` only — a str slice's panic alone
# was +5 KB of core::fmt).
# 85 KB for compile-oom (2026-10-29): the compiler's lists made smaller and
# let go before the layout (the token list counted then kept, the nodes
# and slots reserved once, the new_slot/locals pushes through `try_push`),
# so Endless road's compile fits the ESP32's heap: ~85.6 -> 86,560 B gz
# (+0.9 KB, with the fallible growth and packed tokens left out of the
# wasm build).
# 85 KB for the JIT's estimate (2026-10-29, branch jit-2, opendmd-notes docs/research/
# psi-jit-esp32.md "Phase 2 as built"): `Program::esp32_us_jit` and
# `_jit_still` so the editor's label knows what the panel's JIT draws (its
# cost table `jit_value_ns`, the kernel's first step, a fused colour's
# cost, folded into the estimate's one loop; `info()[13]`): +478 B gz on
# main's 85,622 (86,100). The browser never runs the JIT: the cost table
# only.
# 87 KB with both merged (compile-oom +0.9 KB, the JIT estimate +0.5 KB on 85.6).
# 91 KB for console-lessons Phase B (2026-10-29, branch console-b,
# opendmd-notes docs/research/console-lessons.md §7): `cycle(P, v, s)` (its parse, the
# turn step, the palette slots) and `layer rate N { }` (the slow layers'
# own values in the compiler, the band plan and the from-nothing frame in
# `rate.rs`, the VM's parts), their eleven sentences and the estimate's
# rows: the wasm 83,008 -> 86,639 B gz (+3.6 KB), the line ~87.3 -> ~90.9
# KB.
# 96 KB for the Psi ergonomics first batch (2026-10-30, branch psi-ergo-1,
# opendmd-notes docs/research/psi-ergonomics.md §5): the errors that name a formula's own
# value (P1), `ink(n)` (P3), rebinding (P2), named palette entries (P6)
# and the reworded sentences (P8): the wasm 92,107 B gz on main, the line
# ~90 -> 91 KB at P1 (93,199 B).
PSI_WASM_GZ_BUDGET = 137 * 1024  # 135 until psi-settle (2026-11-04: the language review's A list in the compiler — the one name rule's checks and E215-E218's sentences, the chained-comparison error, the saturation note's DAG intervals (B1), E304's and under()'s sentences (B6) — and the verifier's sprite-pixel check (M3): 136,764 -> 139,123 B gz); 125 until psi-round3 (2026-11-04: Psi ABI 3's built-in inputs, since/was and the 1,024-number store in the compiler, +6.0 KB gz to 133,887 B, and the editor's exports for them — worldReset/worldSet/worldChange/world, sky() with opendmd-core's sky.rs included as it is, builtins() — +2.9 KB gz to 136,764 B); 110 until psi-p0 (2026-11-04: 512 steps, 241 values, palette tweens with their store, scenes and their guards, the counts in E402/E404: the compiler, the verifier the demo links and the estimate, +12.7 KB gz over main's 111.8); 106 until the display-isolation merge (2026-11-03: the demo's Formula look is loaded from the compiled Formula the app sends, `asLookPsb` — the player's psb loader and verifier now linked into the wasm: +4.1 KB gz, 111,183 B); 104 until psi-perf (2026-11-02: the paint primitives costed by the pixels they examine, their boxes from constant values, so the editor's estimate stays the panel's: +0.7 KB gz, 107,018 B); 96 until psi-rebuild (2026-10-31: the scene stage — paint, if, print — and `under(x, y)`, +6.5 KB gz; the player lost ~100 KB of compiler for it); 91 until psi-ergo-1; 87 until console-b; 85 until compile-oom and jit-2 both merged; 84 until compile-oom/jit-2; 83 until formula-apps F2; 82 until main's psi-speed3 merge; 76 until formula-apps F1; 74 until psi-speed2; 73 until palettes; 71 until slot reuse; 67 until sprites; 62 until loop forever; 58 until psi-speed; 36 KB until 2026-10-24 (Psi layers and drawing over a GIF: +4 KB); 40 until parts (one formula inside another); 43 until the text built-ins; 44 until any-look; 45 until the draw stage; 52 until phase 2
# 21 KB for parts (2026-10-25: `use … as p` pasting with its guards, three
# examples). 22 KB for Psi ergonomics
# stage 2 (2026-10-26, branch psi-ergo: the cheat sheet's lines for pick,
# colours and the values about the middle, the editor's colour swatches;
# +192 B gz, the rewritten examples 292 B shorter but only 22 B gz): a small
# raise, with the firmware still comfortable.
# 24 KB for the draw stage (2026-10-27, branch psi-draw): the cheat sheet's
# Drawing section, the highlighter's words, "Draws N lines and dots a frame",
# and two drawing examples (Starfield, now drawn, and Wireframe cube):
# 22,076 -> ~23.1 KB gz.
# 25 KB for pickers and Add to playlist (2026-10-27, branch psi-p2-web): a
# slider with choices (`-- a: shape = cube, ball`) as a row of buttons, in
# js/psikit.js (shared with the site's /f/ page), and Add to playlist on every
# formula (what lets the Library's Effects tab go): 23,614 -> ~24.8 KB gz.
# 27 KB for Psi phase 2 (2026-10-27, branch psi-p2): two more examples
# (Tunnel, Rotozoom: the built-in effects' looks, ~1 KB gz of formulas.json),
# the estimate with the still cache in the ratings, the cheat sheet's
# `pal(fire, v) * k`: 24,811 -> ~26.0 KB gz.
# 28 KB for `loop forever` (2026-10-27, branch psi-lang): its cheat-sheet
# row, the highlighter's `forever`, "Goes on without repeating", Save as
# GIF's first 10 s: 27,612 -> ~27.8 KB gz with Endless road rewritten for it.
# 30 KB for the Time gate example (2026-10-28, branch ct-gate): one more
# example in formulas.json (the CT time gate, 19 lines of code and a dozen
# of comments): 28,435 -> 29,419 B gz. Time gate example; the Formulas split
# (opendmd-notes docs/research/formula-apps.md, pending) will move most examples out of
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
# F3 (most examples out of formulas.json) gives
# ~2.5 KB back and should lower it again.
# 27 KB for formula-apps F3 (2026-10-28, branch formula-f3): the split.
# formulas.json is written by make web from the ten `-- bundle` files in
# opendmd-web/formulas/ (the rest are the site gallery's), as `[{src}]`
# alone (name and about are the sources' own lines): 21,179 B raw / 8,270
# gz -> 4,685 / 2,094; the view's two example cards became one Bundled card
# and a More Formulas link: 32,488 -> 26,584 B gz. Lowered back so the room
# isn't spent by accident (1 KB of headroom, not the old 32).
# 30 KB for the revised bundle (2026-10-28): Endless road and Time
# gate in, Clock and Corner clock out; the two are the largest examples
# (26,584 -> 28,806 B gz, all of it formulas.json).
# 32 KB since formula-refs (2026-10-29): Endless road readable again (names,
# comments: its text no longer has to fit a 2 KB path), +~0.8 KB gz of
# formulas.json; bundled links and their details, +~0.2 KB.
FORMULAS_GZ_BUDGET = 35 * 1024  # 34 until psi-round3 (the editor's built-in inputs: the world drawn with, the import of js/fconds.js and its place; +0.6 KB gz, 34,956 B); 33 until psi-p0 (the cheat sheet's `tween` and `scene` rows: +0.9 KB gz); 32 until f-editor (the site's editor page mounts it alone, Open on my panel's arrival by name or postMessage: ~+0.2 KB); 30 until formula-refs; 27 until Endless road and Time gate were bundled; 32 until formula-apps F3; 31 until formula-apps F2; 30 until formula-apps F1; 28 until Time gate; 27 until loop forever; 16 KB until 2026-10-23 (Psi phase 2: Show on panel), 17 until 2026-10-24 (over a GIF, 8 examples), 20 until parts, 21 until psi-ergo, 22 until the draw stage, 24 until pickers, 25 until phase 2
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
# Your Formulas where they live (on the panel's card, or this browser) and
# the line a playlist or the look names one by (js/fstore.js, branch
# formula-refs, 2026-10-29): imported by the Formulas view and Settings ›
# Look, a line of its own. 3.5 KB since display-isolation met main (main's
# known/read checks and this branch's compiled-file .psb handling together,
# 3,132 B gz, 2026-11-02).
FSTORE_GZ_BUDGET = 3584
FSTORE = {"js/fstore.js"}
# The editor's built-in inputs (js/fconds.js, psi-round3, 2026-11-04: time
# travel, Start a game / Back to the menu / Night, a slider per built-in read;
# imported by js/formulas.js only for a formula that reads one), a line of
# its own, <= 4 KB; and Settings › Schedule › Location (js/location.js:
# latitude, longitude, the browser's location on a click, Clear; imported
# with the tab on a player that keeps one), <= 2 KB. (Measured 3,187 and
# 1,427 B gz.)
FCONDS_GZ_BUDGET = 4 * 1024
FCONDS = {"js/fconds.js"}
LOCATION_GZ_BUDGET = 2 * 1024
LOCATION = {"js/location.js"}
# Submit to the gallery (js/fsubmit.js, branch f-editor, 2026-10-30): the
# form, for the web app's editor and the site's Formula page; imported when
# it first opens (the site's page: once the editor is up). ~3.0 KB gz.
FSUBMIT_GZ_BUDGET = 4 * 1024
FSUBMIT = {"js/fsubmit.js"}
# 8 KB since f-editor (2026-10-30): the page is the web app's editor
# (formulas.js, counted in its own line), with no QR code, no preview of its
# own, no short links and the Submit form in js/fsubmit.js: ~6.0 KB. It was
# 14 (Add to my panel), 13 (short links), 12.
SHARE_GZ_BUDGET = 8 * 1024
# What the formula page loads from the demo (its own files aside).
SHARE_USES = ("pkg/", "pkg-psi/", "js/fcost.js", "js/wasm.js", "js/web.js", "js/build.js", "js/psikit.js", "js/preview.js", "js/state.js", "js/util.js", "js/formulas.js", "js/fstore.js",
              "js/overrides.js", "js/fsubmit.js", "formulas.json", "app.css", "formulas.css", "fonts/silkscreen.woff2")
# Create Art's page on the site (`--share`'s create/: its page, script and
# style; never packed): a line of its own, <= 5 KB (2026-11-02, branch
# create-art: ~2.7 KB for C1, ~3.8 KB with Send to my panel). It is the web app's Studio on the demo's files (js/studio.js,
# js/find.js, the main wasm, pkg-art, ...), counted in their own lines and
# printed as the page's first-visit total for information.
# 8 KB since byo-gif (2026-11-04): the relay's refusals in plain words, ?url=, a page's address, the
# search links and the bookmarklet under Your own, the pick-up of a shared
# file, and the share target's manifest and service worker (sw.js, fetched
# when the page registers it): 7,457 B gz. The app icons (PNG, fetched only when
# the page is installed) are printed, not counted.
# 12 KB since S2 branch 4 (2026-11-11, studio-s2-import; the owner: the site's
# limits are growth alarms, the display's are the real ones): the framed mode
# the Studio's Import sheet runs it in (one action, Use in the Studio, and the
# message to the Studio), the GIF read back into frames (gifread.js, loaded
# at Use), "Start at" for a long video, and the cutover's hand-off into the
# Studio behind its switch: ~12.4 KB gz as written (gifread.js 2.3 of it).
CREATE_GZ_BUDGET = 14 * 1024
# Create Art's video reader (`create/video.js`: an MP4 or WebM played into
# frames for the Studio; site-only, imported only when a video arrives): a
# line of its own, <= 3 KB (2026-11-04, branch byo-gif: ~1.5 KB gz).
VIDEO_GZ_BUDGET = 3 * 1024
VIDEO_JS = "video.js"
CREATE_USES = ("pkg/", "pkg-art/", "js/wasm.js", "js/web.js", "js/build.js", "js/state.js", "js/util.js", "js/preview.js", "js/artmod.js", "js/studio.js", "js/find.js", "js/importgif.js",
               "js/gifs.js", "js/details.js", "js/prepare.js", "app.css", "studio.css", "find.css", "fonts/silkscreen.woff2")
# The Studio's page on the site (`--share`'s studio/: the pixel editor, its
# page, script, style, the record store and the starter palettes; site-only,
# never packed): a line of its own, <= 20 KB (2026-11-11, branch studio-s0:
# the S0 spike, 17.4 KB gz as written here, comments and all; the site serves
# it minified). Its GIF writer was the demo's pkg-art until S2 (now
# pkg-studio, the studio wasm line below), fetched at the first download,
# not with the page.
# 32 KB since S1 (2026-11-11, branch studio-s1, under the budget authority:
# ~30.2 KB gz as written): the line, rectangle, ellipse, dither, selection
# and picker tools with their live previews (tools.js), brushes, the piece's
# own colours with two-byte keys, the undo journal kept across reloads,
# snapshots, the tab-safe store version, pinch zoom. What can wait is a line
# of its own below; the site serves both minified (about half this).
# 40 KB since S2 (2026-11-11, branch studio-s2-doc, under the budget
# authority, as the S2 plan has it): first paint is only what a one-layer
# still needs plus the model that opens and shows any piece (doc.js: cels by
# layer and frame, the composite, the colour keys, the undo steps), the
# seven files of STUDIO_FIRST; the GIF went out to out.js.
# 42 KB since S2 branch 2 (2026-11-11, studio-s2-frames, under the budget
# authority the coordinator passed on): the first-paint wiring for frames and
# layers (doc.js applying and undoing structure steps, which a journal can
# replay before frames.js is in; the strip's one cell and "+", the Layers
# section, the phone's tabs, Send to panel's button, the kind line) at
# 42,827 B gz as written (after the review round: mid-stroke guards, the
# phone layout, the keys behind a sheet). The frames code itself is lazy (frames.js).
# 48 KB since S2 branch 3 (2026-11-11, studio-s2-library; the owner: the
# site's limits matter less than the display's, so these lines are growth
# alarms): the wiring for the Library, files and stamps (the Library
# button and #library view, the Stamp tool and Save as stamp, the Stamps
# section and its phone tab, This piece's Save as file / Open a file and
# what it was made from), check() reading stamps, fonts, templates and
# packs, the starter pack's every record: ~45.3 KB gz as written.
# 50 KB since S2 branch 4 (2026-11-11, studio-s2-import, the same authority):
# the Import… button and #import route, the paste event (an image to
# paste.js, an address to Import, else the Studio's own copy), Copy's PNG on
# the system clipboard, drops routed (Studio files to files.js, pictures and
# videos to Import), a whole body as one step for an import's layer:
# ~47.6 KB gz as written.
# 52 KB since studio-share (2026-11-12, the coordinator's OK, site-only): the
# one notice pinned in view (say/offer: under its control or at the top, in
# the open sheet, Escape and Dismiss, two live regions, focus back):
# ~50.4 KB gz as written.
# 54 KB since studio-float F0 (2026-11-13, the owner's OK, site-only): the one-screen
# workspace from 1000 x 620 up (FLOAT.md: the tools a column, the bench a scrolling column, the
# frames under the stage, zoom and the guides in the chin, the bench's width set so Fit lands on
# a whole step) and Space pressing a focused button rather than playing: ~52.9 KB gz as written.
STUDIO_PAGE_GZ_BUDGET = 54 * 1024
STUDIO_FIRST = ("index.html", "studio.css", "studio.js", "doc.js", "store.js", "palettes.js", "tools.js")
# What the Studio's page imports only when it is first used: a line each
# (S2 plan, 2026-11-11), name -> (gz budget, its files). Every file in
# studio/ is first paint or on one of these lines; one on neither fails the
# build, so nothing joins the page uncounted. A line whose files are not
# written yet prints as such and passes. colour.js (the picker, the palette
# tools, the Lospec import) was the one lazy line, 5 KB, in S1 (~3.8 KB gz).
# The starter pack the page fetches once it is up
# (studio-starter/starter.dmdpack, copied beside it by tools/site.py) is
# content, not code, and counts against none of them.
STUDIO_LAZY = {
    "colour": (6 * 1024, ("colour.js",)),        # New colour…, Palettes…
    "frames": (7 * 1024, ("frames.js", "frames.css")),  # more than one frame or layer, Add frame, Add layer, Play
    "out": (5 * 1024, ("out.js",)),              # Download, Send to panel, Share: flatten, the GIF table
    # files 10 KB (was 5, then 8; branch 3 and its reviews): every foreign record checked and hashed, packs walked for
    # cycles, the merge shown before anything is stored, backups split to open, copies found across the files of one opening.
    "files": (10 * 1024, ("files.js",)),          # Save as file, Open a file, a drop, Back up
    "library": (10 * 1024, ("library.js", "library.css")),  # Library (10 KB, was 8: branch 3's detail, duplicate, templates)
    "stamps": (5 * 1024, ("stamps.js",)),        # the Stamp tool, Save as stamp (5 KB, was 4: the ghost and the two sets)
    # link 6 KB (4 in the plan; branch 5, ~5.2 KB): the #p= reader checks every byte against what came before it, the preview plays.
    "link": (6 * 1024, ("link.js",)),            # Share, or an address with #p= or #k=
    # paste 3 KB (was 2): a picture's size read from its first bytes (PNG, GIF, JPEG, WebP, BMP) before it is decoded.
    "paste": (3 * 1024, ("paste.js",)),          # a paste that carries an image
    # import 6 KB (3 in the plan; branch 4, ~4.9 KB): the sheet and its frame, every message checked, frames to a piece or a layer.
    "import": (6 * 1024, ("import.js",)),        # Import…, a picture or video dropped, a paste too big to place, #import
    # panels 10 KB (studio-float F1, 2026-11-13; ~4 in FLOAT.md before the owner added resizing and hide-all): the
    # workspace's panels, docked or floating, dragged, moved by keys, resized, folded, closed, hidden all at once, kept.
    # 11 KB since studio-dark (2026-11-13, site-only, ~10.5 KB): the row of panels under the canvas, a third dock (its room
    # worked out from the window, kept through a drag, its place in the menus).
    "panels": (11 * 1024, ("panels.js", "panels.css")),  # a window of the workspace's size (1000 x 620 or more)
}
# Everything Import's frame fetches the first time it opens (S2 plan,
# 2026-11-11, branch studio-s2-import): Create Art's page in its framed mode
# and the demo's files it runs on (the main wasm, pkg-art, the Make art
# editor and its styles), and the Studio's import.js. ~303 KB gz on the built
# site when planned; counted here as written for the page's own files.
IMPORT_OPEN_GZ_BUDGET = 330 * 1024
# First paint and every lazy line together. 92 KB since S2 branch 4 (was 84;
# import.js and paste.js, as their lines).
# 96 KB since S2 branch 5 (studio-s2-links: link.js, the pack's Share link, ~92.7 KB).
# 98 KB since studio-share (2026-11-12, the notice above, ~96.2 KB).
# 110 KB since studio-float (2026-11-13, the owner's OK for 104 with "raise to what resizing and hide-all
# need"): the workspace (F0) and the panels line (F1).
STUDIO_ALL_GZ_BUDGET = 110 * 1024
STUDIO_PAGE_USES = ("pkg-studio/", "js/wasm.js", "js/build.js", "fonts/silkscreen.woff2")
# The Studio page's GIF writers (pkg-studio/: opendmd-studio-web, the
# indexed, wide and fitted encoders over the gif crate; site-only, left out
# of the pack by opendmd-pack www.rs SITE_ONLY): a line of its own, <= 32 KB
# (2026-11-11, branch studio-encoders, under the budget authority: 29,253 B
# gz; 40 KB at first, lowered in studio-enc-tidy). They were in pkg-art
# (S1), which also ships in the device's www pack and had ~0.5 KB of its
# 150 KB left; moved out, pkg-art is at 147,638 B gz, 2.4 KB under it.
STUDIO_WASM_GZ_BUDGET = 32 * 1024
LICENCES_GZ_BUDGET = 40 * 1024
LICENCES = {"licenses.txt"}
# The web demo's player (js/demo.js: the settings API answered in the page,
# the schedule, what settings do to the demo's panel): imported by app.js
# only for the site's demo and left out of the pack (opendmd-pack www.rs
# SITE_ONLY), so outside the 60 KB line.
# 9 KB since formula-refs (2026-10-29): a look named `@formula:<stem>`
# resolved from formulas.json, as the player has the bundled ones built in.
DEMO_GZ_BUDGET = 9 * 1024
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
# The playlist editor's "Add by filter" sheet (js/plbuild.js, branch tags,
# 2026-11-10: the search it builds, the tag chips and stars, "Keep this
# search"; imported when it first opens, so the editor's line shrank by the
# sheet), and smart playlists (js/smart.js: /sets/.smart, a playlist written
# again from its search; imported by Playlists and the sheet), and the
# user's tags and stars (js/tags.js: /sets/.tags held in the wasm's TagBook,
# the Library's rows, Details and the bulk Tag/Rate sheets; imported with the
# Library), and the references a move, rename or delete rewrites (js/refs.js:
# playlists, their favourites, tags, Smooth motion; the bulk Move and Rename
# dialogs; imported when first used): a line each.
PLBUILD_GZ_BUDGET = 4 * 1024
PLBUILD = {"js/plbuild.js"}
SMART_GZ_BUDGET = 3 * 1024
SMART = {"js/smart.js"}
TAGS_GZ_BUDGET = 6 * 1024
TAGS = {"js/tags.js"}
REFS_GZ_BUDGET = 4 * 1024
REFS = {"js/refs.js"}
# The error codes of the Formulas editor's messages (`js/psierr.js`: the
# compiler's sentence fragments for each code, generated from
# opendmd-psi/src/errors.rs, and the link to the language reference's
# entry; imported the first time a formula shows an error, by js/formulas.js
# and the site's /f/ page): a line of its own, <= 4 KB (2026-10-30, branch
# ref-ergo: 3,001 B gz for 137 codes). Not with the Formulas line (1.3 KB
# left) nor the page's 60 KB line, as it never loads with either.
PSIERR_GZ_BUDGET = 4 * 1024
PSIERR = {"js/psierr.js"}
# Earlier versions of a Formula (`js/fhist.js`, branch editor-save,
# 2026-11-07: the ring of the last 10 per Formula in this browser, the line
# diff, its menu, and the site's /f/ draft and Continue where you left off),
# imported by js/formulas.js on a save, an arrival or the menu, and by /f/
# as it starts: a line of its own, <= 3 KB, not the Formulas line.
FHIST_GZ_BUDGET = 3 * 1024
FHIST = {"js/fhist.js"}
# The editor's names at the caret (`js/fhint.js`: completion of the
# built-in and the formula's own names, a call's values with the one being
# typed in bold, and the cheat sheet, whose rows moved here from
# js/formulas.js; generated from opendmd-psi/tests/builtins.rs; imported at
# the editor's first key or the Cheat sheet's first click): a line of its
# own (2026-11-07, branch editor-hint), apart from the Formulas line, which
# every visit to the view loads and which the cheat sheet's rows left
# (23,297 B gz after, from 25,959). Measured 6,598 B gz: ~3.4 KB the
# cheat sheet's rows, the rest the names and the list.
# 8 KB (2026-11-11, Claude, under the owner's budget authority; branch
# psi-tidy): `phase` and `smooth` rows and names took it to 7,065 B gz,
# 103 B of headroom left.
FHINT_GZ_BUDGET = 8 * 1024
FHINT = {"js/fhint.js"}
# The site editor's What costs most and Tidy up (2026-11-07, branch
# editor-cost): js/fcost.js (imported by js/formulas.js only on the site's
# /f/ and demo) and its wasm, pkg-psi-lang/ (opendmd-psi-web's `lang`
# build: observe() and format() on the compiler with its `syntax` hook),
# both left out of the pack (opendmd-pack www.rs SITE_ONLY). The wasm is
# loaded only when a Formula is past light or Tidy up is pressed. It is the
# whole compiler (the observations need it), 121,347 B gz with its glue: the same module
# with the device's exports too would be ~154 KB (pkg-psi + ~24.6 KB).
FCOST_GZ_BUDGET = 3 * 1024
FCOST = {"js/fcost.js"}
PSI_LANG_GZ_BUDGET = 124 * 1024
# The Formula editor's picture and palette editors (js/fsprite.js, branch
# editor-sprite, 2026-11-07: a sprite's grid to paint from its key, a
# palette's strip as the panel reads it with a handle per colour; imported
# by js/formulas.js only for a formula with a sprite block or a palette
# line): a line of its own, <= 6 KB (5,661 B gz at first, its styles
# with it), as it never loads with the Formulas line.
FSPRITE_GZ_BUDGET = 6 * 1024
FSPRITE = {"js/fsprite.js"}
# A sprite from a picture (js/fimport.js, branch sprite-import, 2026-11-13:
# the picture read in the browser, an SVG made safe, the crop, the size,
# the colours reduced to a key of 2, 4 or 16, the dialog and its cost line;
# imported by js/formulas.js only once a picture is chosen, dropped or
# pasted): a line of its own, as it never loads with the Formulas line or
# fsprite's (it imports fsprite.js's parsers). Planned at 6 KB; 10 KB
# (Claude, under the owner's budget authority): measured ~9.3 KB gz with
# the SVG sanitiser, the median cut, the re-import's key matching and the
# dialog's crop, a file only a picture's import ever fetches (the pack
# keeps > 170 KB spare).
FIMPORT_GZ_BUDGET = 10 * 1024
FIMPORT = {"js/fimport.js"}
# The site's Psi pages' script (`--share`'s ref/psiref.js, copied to the
# site's root and loaded by the reference, its error codes and the guide's
# Psi pages: the live examples, highlighting, the name index, search and the
# cheat sheet; never packed): a line of its own, <= 6 KB (2026-10-30, branch
# ref-ergo: ~4.1 KB gz; branch psi-machinery, every page's ```psi blocks,
# row ids and the index, the old #E addresses sent on: unchanged budget),
# apart from /f/'s share line, which it never loads with. It runs on the demo's pkg-psi and js/psikit.js, counted in their
# own lines, loaded only when the first example scrolls near. 7 KB from
# psi-site (2026-11-08): measured unminified with psiled.js, ~6.1 KB; the
# minified /psiref.js served is ~2.8 KB gz.
PSIREF_GZ_BUDGET = 7 * 1024
# The site's other live Formulas, on the same psiled.js: the home page's
# panels (ref/home.js) and the gallery's tiles on hover (ref/ftiles.js),
# a line each with psiled.js, <= 4 KB (2026-10-30: ~3.1 KB gz each).
LIVE_REF = {"home.js": "home", "ftiles.js": "gallery"}
LIVE_REF_GZ_BUDGET = 4 * 1024
SHARED_REF = "psiled.js"
# The gallery's sorting and filtering (ref/ftsort.js, beside ftiles.js; no
# Psi): a line of its own, <= 3 KB (2026-11-02, branch gallery-sort: ~2.3 KB gz).
FTSORT = "ftsort.js"
FTSORT_GZ_BUDGET = 3 * 1024
# The home page's hero show (ref/hero.js, beside home.js, which imports it
# after the page has loaded; on psiled.js, counted with home.js): the
# panel's transitions (opendmd-core compositor.rs), a GIF decoder for the
# site's own art and the little playlist. A line of its own, <= 5 KB
# (2026-11-04, branch site-demo: ~4.6 KB gz, about half of it the
# transitions and the GIF decoder; it loads after the page has).
HERO = "hero.js"
HERO_GZ_BUDGET = 5 * 1024
# The web app's LEDs over the home page's screenshots whose panel plays
# (ref/leds.js, imported by home.js as the first nears the screen): a line
# of its own, <= 2 KB (2026-11-13, branch road-wording: ~1.2 KB gz).
LEDS = "leds.js"
LEDS_GZ_BUDGET = 2 * 1024


# The signature trailer `fw-sign.py sign-www` appends (docs/www-pack.md).
WWW_SIGNATURE_BYTES = 72


def q_ref(p: Path, share: Path) -> bool:
    """A file of the Psi reference's script (share/ref/), not /f/'s."""
    return p.relative_to(share).parts[0] == "ref"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("root", nargs="?", default="opendmd-web/www-dist")
    ap.add_argument("--pack", help="the www pack built from it")
    ap.add_argument("--pack-limit", type=int, default=786_432, help="the www partition, bytes (firmware/partitions.csv)")
    ap.add_argument("--pack-spare", type=int, default=65_536, help="what the signed pack must leave of the partition, bytes")
    ap.add_argument("--share", help="the site's formula page sources (opendmd-web/share)")
    ap.add_argument("--unminified", action="store_true",
                    help="a WEB_MINIFY=0 build: print the lines, enforce none")
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
        elif rel.startswith("pkg-psi-lang/"):
            kind = "psi lang"
        elif rel.startswith("pkg-studio/"):
            kind = "studio wasm"
        elif rel in FCOST:
            kind = "fcost"
        elif rel in FORMULAS:
            kind = "formulas"
        elif rel in SHOW:
            kind = "show"
        elif rel in LOOKS:
            kind = "looks"
        elif rel in FSHEET:
            kind = "fsheet"
        elif rel in FSTORE:
            kind = "fstore"
        elif rel in FCONDS:
            kind = "fconds"
        elif rel in LOCATION:
            kind = "location"
        elif rel in FSUBMIT:
            kind = "fsubmit"
        elif rel in STUDIO:
            kind = "studio"
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
        elif rel in KEYNAG:
            kind = "keynag"
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
        elif rel in PLBUILD:
            kind = "plbuild"
        elif rel in SMART:
            kind = "smart"
        elif rel in TAGS:
            kind = "tags"
        elif rel in REFS:
            kind = "refs"
        elif rel in PSIERR:
            kind = "psierr"
        elif rel in FHIST:
            kind = "fhist"
        elif rel in FHINT:
            kind = "fhint"
        elif rel in FSPRITE:
            kind = "fsprite"
        elif rel in FIMPORT:
            kind = "fimport"
        elif rel in IMPORT:
            kind = "import"
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
        rraw = rgz = 0
        live: dict[str, tuple[int, int]] = {}
        shared = (0, 0)
        ftsort = (0, 0)
        hero = (0, 0)
        leds = (0, 0)
        craw = cgz = 0
        sp_raw = sp_gz = 0
        sp_seen: set[str] = set()
        lazy = {name: [0, 0, []] for name in STUDIO_LAZY}
        lazy_of = {f: name for name, (_, fs) in STUDIO_LAZY.items() for f in fs}
        stray: list[str] = []
        video = (0, 0)
        for p in sorted(q for q in share.rglob("*") if q.is_file() and "tests" not in q.relative_to(share).parts):
            if q_ref(p, share):
                b = p.read_bytes()
                g = len(gzip.compress(b, 9, mtime=0))
                print(f"{'share/' + p.relative_to(share).as_posix():<40} {len(b):>9,} {g:>9,}")
                if p.name in LIVE_REF:
                    live[LIVE_REF[p.name]] = (len(b), g)
                    continue
                if p.name == FTSORT:
                    ftsort = (len(b), g)
                    continue
                if p.name == HERO:
                    hero = (len(b), g)
                    continue
                if p.name == LEDS:
                    leds = (len(b), g)
                    continue
                if p.name == SHARED_REF:
                    shared = (len(b), g)
                rraw += len(b)
                rgz += g
                continue
            b = p.read_bytes()
            g = len(gzip.compress(b, 9, mtime=0))
            print(f"{'share/' + p.relative_to(share).as_posix():<40} {len(b):>9,} {g:>9,}")
            if p.relative_to(share).parts[0] == "create" and p.suffix == ".png":
                continue
            if p.relative_to(share).parts[0] == "create" and p.name == VIDEO_JS:
                video = (len(b), g)
                continue
            if p.relative_to(share).parts[0] == "studio":
                sub = "/".join(p.relative_to(share).parts[1:])
                if sub in STUDIO_FIRST:
                    sp_raw += len(b)
                    sp_gz += g
                    sp_seen.add(sub)
                elif sub in lazy_of:
                    t = lazy[lazy_of[sub]]
                    t[0] += len(b)
                    t[1] += g
                    t[2].append(sub)
                else:
                    stray.append(sub)
                continue
            if p.relative_to(share).parts[0] == "create":
                craw += len(b)
                cgz += g
                continue
            sraw += len(b)
            sgz += g

        def demo_gz(prefixes: tuple[str, ...]) -> int:
            return sum((q.with_name(q.name + ".gz").stat().st_size if q.with_name(q.name + ".gz").exists() else q.stat().st_size)
                       for q in files if any(q.relative_to(root).as_posix().startswith(u) for u in prefixes))
        uses = demo_gz(SHARE_USES)
        verdict = "ok" if sgz <= SHARE_GZ_BUDGET else "OVER BUDGET"
        print(f"{'share':<10} raw {sraw:>9,}           gz {sgz:>9,} / {SHARE_GZ_BUDGET // 1024} KB {verdict:<10}"
              f"(first visit with the demo's files: {sgz + uses:,} B gz)")
        verdict = "ok" if cgz <= CREATE_GZ_BUDGET else "OVER BUDGET"
        print(f"{'create':<10} raw {craw:>9,}           gz {cgz:>9,} / {CREATE_GZ_BUDGET // 1024} KB {verdict:<10}"
              f"(first visit with the demo's files: {cgz + demo_gz(CREATE_USES):,} B gz)")
        ok &= cgz <= CREATE_GZ_BUDGET
        open_gz = cgz + demo_gz(CREATE_USES) + lazy["import"][1]
        verdict = "ok" if open_gz <= IMPORT_OPEN_GZ_BUDGET else "OVER BUDGET"
        print(f"{'import open':<12} raw {'':>9}         gz {open_gz:>9,} / {IMPORT_OPEN_GZ_BUDGET // 1024} KB {verdict:<10}(what the Studio's Import fetches when it first opens)")
        ok &= open_gz <= IMPORT_OPEN_GZ_BUDGET
        missing = [f for f in STUDIO_FIRST if f not in sp_seen]
        verdict = "ok" if 0 < sp_gz <= STUDIO_PAGE_GZ_BUDGET and not missing else "OVER BUDGET" if not missing else "MISSING"
        print(f"{'studio pg':<10} raw {sp_raw:>9,}           gz {sp_gz:>9,} / {STUDIO_PAGE_GZ_BUDGET // 1024} KB {verdict:<10}"
              f"(first paint, {len(sp_seen)} files{'; missing ' + ', '.join(missing) if missing else ''}; with the GIF writer and out.js,"
              f" at the first download: {sp_gz + lazy['out'][1] + demo_gz(STUDIO_PAGE_USES):,} B gz)")
        ok &= 0 < sp_gz <= STUDIO_PAGE_GZ_BUDGET and not missing
        all_gz = sp_gz
        for name, (budget, fs) in STUDIO_LAZY.items():
            raw_, gz_, got = lazy[name]
            all_gz += gz_
            verdict = "ok" if gz_ <= budget else "OVER BUDGET"
            print(f"{'studio ' + name:<14} raw {raw_:>9,}       gz {gz_:>9,} / {budget // 1024} KB {verdict:<10}"
                  f"({', '.join(got) if got else ', '.join(fs) + ': not written yet'})")
            ok &= gz_ <= budget
        verdict = "ok" if all_gz <= STUDIO_ALL_GZ_BUDGET else "OVER BUDGET"
        print(f"{'studio all':<10} raw {'':>9}           gz {all_gz:>9,} / {STUDIO_ALL_GZ_BUDGET // 1024} KB {verdict:<10}(first paint and every lazy line)")
        ok &= all_gz <= STUDIO_ALL_GZ_BUDGET
        for f in stray:
            print(f"studio UNLISTED studio/{f}: every file in studio/ is first paint (STUDIO_FIRST) or on a lazy line (STUDIO_LAZY) in tools/web-size.py")
        ok &= not stray
        verdict = "ok" if 0 < video[1] <= VIDEO_GZ_BUDGET else "OVER BUDGET"
        print(f"{'video':<10} raw {video[0]:>9,}           gz {video[1]:>9,} / {VIDEO_GZ_BUDGET // 1024} KB {verdict}")
        ok &= 0 < video[1] <= VIDEO_GZ_BUDGET
        verdict = "ok" if rgz <= PSIREF_GZ_BUDGET else "OVER BUDGET"
        print(f"{'psiref':<10} raw {rraw:>9,}           gz {rgz:>9,} / {PSIREF_GZ_BUDGET // 1024} KB {verdict}")
        for name, (raw, gz) in sorted(live.items()):
            raw, gz = raw + shared[0], gz + shared[1]
            verdict = "ok" if gz <= LIVE_REF_GZ_BUDGET else "OVER BUDGET"
            print(f"{name:<10} raw {raw:>9,}           gz {gz:>9,} / {LIVE_REF_GZ_BUDGET // 1024} KB {verdict}")
            ok &= gz <= LIVE_REF_GZ_BUDGET
        verdict = "ok" if ftsort[1] <= FTSORT_GZ_BUDGET else "OVER BUDGET"
        print(f"{'ftsort':<10} raw {ftsort[0]:>9,}           gz {ftsort[1]:>9,} / {FTSORT_GZ_BUDGET // 1024} KB {verdict}")
        ok &= ftsort[1] <= FTSORT_GZ_BUDGET
        verdict = "ok" if hero[1] <= HERO_GZ_BUDGET else "OVER BUDGET"
        print(f"{'hero':<10} raw {hero[0]:>9,}           gz {hero[1]:>9,} / {HERO_GZ_BUDGET // 1024} KB {verdict}")
        ok &= 0 < hero[1] <= HERO_GZ_BUDGET
        verdict = "ok" if leds[1] <= LEDS_GZ_BUDGET else "OVER BUDGET"
        print(f"{'leds':<10} raw {leds[0]:>9,}           gz {leds[1]:>9,} / {LEDS_GZ_BUDGET // 1024} KB {verdict}")
        ok &= 0 < leds[1] <= LEDS_GZ_BUDGET
        print()
        ok &= sgz <= SHARE_GZ_BUDGET and rgz <= PSIREF_GZ_BUDGET
    for kind, budget in (("wasm+glue", WASM_GZ_BUDGET), ("js+css", JS_CSS_GZ_BUDGET),
                         ("art wasm", ART_WASM_GZ_BUDGET), ("studio", STUDIO_GZ_BUDGET),
                         ("boot", BOOT_GZ_BUDGET),
                         ("power", POWER_GZ_BUDGET), ("details", DETAILS_GZ_BUDGET), ("import", IMPORT_GZ_BUDGET), ("find", FIND_GZ_BUDGET),
                         ("smooth", SMOOTH_GZ_BUDGET), ("firmware", FIRMWARE_GZ_BUDGET), ("update", UPDATE_GZ_BUDGET), ("card", CARD_GZ_BUDGET),
                         ("panel", PANEL_GZ_BUDGET), ("panel test", PANEL_TEST_GZ_BUDGET), ("keynag", KEYNAG_GZ_BUDGET), ("device", DEVICE_GZ_BUDGET),
                         ("psi wasm", PSI_WASM_GZ_BUDGET), ("formulas", FORMULAS_GZ_BUDGET), ("show", SHOW_GZ_BUDGET), ("looks", LOOKS_GZ_BUDGET),
                         ("fsheet", FSHEET_GZ_BUDGET), ("fstore", FSTORE_GZ_BUDGET), ("fsubmit", FSUBMIT_GZ_BUDGET),
                         ("fconds", FCONDS_GZ_BUDGET), ("location", LOCATION_GZ_BUDGET),
                         ("font", FONT_GZ_BUDGET), ("licences", LICENCES_GZ_BUDGET), ("demo", DEMO_GZ_BUDGET),
                         ("tree", TREE_GZ_BUDGET), ("editor", EDITOR_GZ_BUDGET), ("plbuild", PLBUILD_GZ_BUDGET), ("smart", SMART_GZ_BUDGET), ("tags", TAGS_GZ_BUDGET), ("refs", REFS_GZ_BUDGET), ("psierr", PSIERR_GZ_BUDGET), ("fhist", FHIST_GZ_BUDGET), ("fhint", FHINT_GZ_BUDGET),
                         ("fcost", FCOST_GZ_BUDGET), ("psi lang", PSI_LANG_GZ_BUDGET), ("studio wasm", STUDIO_WASM_GZ_BUDGET), ("fsprite", FSPRITE_GZ_BUDGET),
                         ("other", None)):
        raw, gz = totals.get(kind, [0, 0])
        verdict = "" if budget is None else ("ok" if gz <= budget else "OVER BUDGET")
        limit = "" if budget is None else f"/ {budget // 1024} KB"
        print(f"{kind:<10} raw {raw:>9,}           gz {gz:>9,} {limit:<10} {verdict}")
        ok &= budget is None or gz <= budget
    # (The picture import's line, on its own: js/fimport.js.)
    raw, gz = totals.get("fimport", [0, 0])
    verdict = "ok" if 0 < gz <= FIMPORT_GZ_BUDGET else "OVER BUDGET" if gz else "MISSING"
    print(f"{'fimport':<10} raw {raw:>9,}           gz {gz:>9,} / {FIMPORT_GZ_BUDGET // 1024} KB {'':<5}{verdict}")
    ok &= 0 < gz <= FIMPORT_GZ_BUDGET
    all_raw = sum(t[0] for t in totals.values())
    all_gz = sum(t[1] for t in totals.values())
    print(f"{'total':<10} raw {all_raw:>9,}           gz {all_gz:>9,}")
    if args.pack:
        n = Path(args.pack).stat().st_size + WWW_SIGNATURE_BYTES
        spare = args.pack_limit - n
        verdict = "ok" if spare >= args.pack_spare else "OVER BUDGET"
        print(f"pack: {n:,} B signed of {args.pack_limit // 1024:,} KB ({n * 100 / args.pack_limit:.1f}%), "
              f"{spare:,} B spare (need {args.pack_spare:,}) {verdict}")
        ok &= spare >= args.pack_spare
    if args.unminified:
        print("unminified build (WEB_MINIFY=0): the budget lines are for minified files and are not enforced"
              + ("" if ok else " (some are over)"))
        return 0
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
