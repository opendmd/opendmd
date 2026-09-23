# Art pipeline: 128x32 art for your own games, made in the browser

Status: investigation and throwaway prototypes, 2026-09-23. Nothing here is
in the app yet. The prototypes are in `opendmd-web/experiments/art/`
(`README.md` there says how to run each one). All sample inputs are CC0 or
public domain, so the committed PNGs hold no third-party game art.

**The brief.** The owner wants panel art for the consoles and games he owns
(Mega Drive, SNES, Game Boy, Saturn, 3DS). The pack is thin on GB (14 files),
Saturn (34) and has nothing for 3DS. The art is for personal use at home. The
runtime is the web app: pull art from the web, process it in the browser,
save it to the panel. The output is a GIF filed by the tag grammar
(`SNES_Title_NN.gif`). The existing Add GIFs path then uploads it and
prepares its copy.

## Summary

- **Sources.** The CORS test sorts the hosts into two groups.
  - Readable from a page: Wikimedia/Wikipedia (with `origin=*`),
    raw.githubusercontent.com, Giphy's CDN, the IGDB and RAWG image CDNs,
    archive.org, and the Tenor, Giphy, Lospec and ScreenScraper APIs.
  - Not readable: OpenGameArt, The Spriters Resource, itch.io, SteamGridDB,
    TheGamesDB, the IGDB/RAWG/SteamGridDB APIs, and
    thumbnails.libretro.com.
- **The best single source is libretro-thumbnails, fetched through
  raw.githubusercontent.com.** It has title screens, in-game snaps and logos
  for every one of the five systems, named by No-Intro game name. MD, SNES
  and GB are stored at native resolution, and it needs no key. RePlayOS's
  `get_status` gives `system` and `game_name`, which is enough to look one up.
- **Escape hatches work without CORS or HTTPS.** File picker, drag-and-drop
  and the `paste` event all work over the device's plain-http LAN origin.
  `navigator.clipboard.read()` and Web Share Target do not (both need a
  secure context), and iOS has no share target at all.
- **Emulator in the browser.** This works and gives native-resolution
  frames: GB 160x144, MD 320x224.
  - **binjgb** (MIT, Game Boy only) is 102 KB, reaches its first frame in
    10 ms, and grabs a frame in 0.2 ms.
  - **EmulatorJS** (GPL-3.0 RetroArch plus per-core licences, two of them
    non-commercial) is 1.2–2 MB per system and takes about 0.8–4 s to start.
  - So: ship binjgb, and make the other systems an optional core the user
    loads from a CDN or drops on the card. Never bundle cores in the repo.
  - Saturn in the browser is untested and assumed slow. 3DS is not viable.
- **Processing.** Keep native pixels. Detect and undo 2x/3x upscales, then
  pick a 128x32 window at 1:1. For 2:1, use a "mode" downscale, never area
  averaging. Palettes should come from the source's own colours, picked for
  spread. Median cut and NeuQuant (the core's quantiser) both drop small
  accents: the yellow coin, the blue gem, the Saturn planet, the green "D" of
  the Mega Drive logo. Dithering stays off. Defaults are 40 ms frames,
  1 px/frame for pans and scrolls, and 1.5 s holds at the ends.
- **Output.** Save one global palette of 256 colours or fewer. Then the
  prepared copy (`encodeGif`) keeps the colours exactly and never
  re-quantises. The prototype checked this for every output: `quantised:
  false`, and `opendmd-pack --verify` gives 5/5 identical.
- **First milestone.** A "Make art" sheet in Library with Import (file, drop,
  paste, URL, libretro-thumbnails lookup by platform + title), Frame (auto
  window, drag, still/pan/scroll), Preview (the existing LED `Preview`) and
  Save (grammar filename → Add GIFs path). It needs no keys and no proxy. The
  estimate is 4–6 days, detailed at the end.

---

## 1. Sources, and how a page reaches them

### The CORS results

`experiments/art/cors/cors_probe.py` serves a page from
`http://127.0.0.1:<port>/`, a real http origin like the device's. It then
runs three checks against 31 live URLs:

- `fetch(url, {mode:'cors'})`
- `<img crossOrigin=anonymous>` → canvas → `getImageData`
- a plain `<img>` → canvas, to confirm the canvas gets tainted

The raw results are in `cors/cors_results.json`. Where Chromium hid the
response of a failed fetch, the headers were confirmed with curl.

| source | URL kind | fetch | ACAO | pixels readable |
|---|---|---|---|---|
| Wikimedia Commons API, `&origin=*` | API | 200 | `*` | — |
| Wikimedia Commons API, no `origin` | API | blocked | none | — |
| thumb.wikimedia.org PNG thumbnail | image | 200 | `*` | **yes** |
| upload.wikimedia.org original | image | 200 | `*` | **yes** |
| Wikipedia Action API `origin=*` / REST `page/summary` | API | 200 | `*` | — |
| OpenGameArt `sites/default/files/…` | image | blocked | none | no (tainted) |
| Lospec palette JSON | API | 200 | `*` | — |
| Lospec gallery (cdn.lospec.com) | image | blocked | none | no |
| The Spriters Resource CDN | image | blocked | none | no |
| Tenor v2 search (no key / dead demo key) | API | 403 / 400 | reflects `Origin` | — |
| Giphy search (no key / retired beta key) | API | 401 / 403 | `*` | — |
| Giphy media CDN | image | 200 | `*` | **yes** |
| IGDB API `api.igdb.com/v4` | API | blocked | none | — |
| IGDB images `images.igdb.com` | image | 200 | `*` | **yes** |
| TheGamesDB API + `cdn.thegamesdb.net` | both | blocked | none | no |
| ScreenScraper `api2/jeuInfos.php` (no auth) | API | 400 | `*` | — |
| RAWG API | API | blocked | none | — |
| RAWG images `media.rawg.io` | image | 200 | `*` | **yes** |
| itch.io `img.itch.zone` | image | blocked | none | no |
| SteamGridDB API + `cdn2.steamgriddb.com` | both | blocked | none | no |
| raw.githubusercontent.com (a libretro-thumbnails PNG) | image | 200 | `*` | **yes** |
| api.github.com contents / trees | API | 200 | `*` | — |
| thumbnails.libretro.com (same files) | image | blocked | none | no |
| archive.org `/metadata`, `/download` (302 to a node) | both | 200 | `*` | **yes** |
| images.weserv.nl (public image proxy) | image | 200 | `*` | **yes** |

Three things were surprising:

- **Several hosts split API from CDN.** IGDB's and RAWG's APIs send no ACAO,
  but their image CDNs do. The fix is a proxied (or keyed, server-side)
  metadata call and a direct image load.
- **libretro-thumbnails: the host decides.** `thumbnails.libretro.com` has
  no CORS. The same files through
  `raw.githubusercontent.com/libretro-thumbnails/<System>/master/…` do.
- **Some APIs send ACAO even on errors.** ScreenScraper sends it with a 400,
  Giphy with 401/403, and Tenor reflects the Origin with a 403. So
  "blocked" has to be read from the header, not from the status.

One run in about five saw a transient `Failed to fetch` from the Commons API
from headless Chromium. The next run was fine. The app should retry once and
show the error as a message.

### Per source

**libretro-thumbnails (recommended first source).** These are GitHub repos,
one per system, used by RetroArch. Each has `Named_Titles/`,
`Named_Snaps/`, `Named_Boxarts/` and `Named_Logos/`, and every file is named
by its No-Intro/Redump game name. Verified 2026-09-23 with the git trees API
(one request per system, not truncated):

| repo | titles | snaps | logos | boxarts |
|---|---|---|---|---|
| `Sega_-_Mega_Drive_-_Genesis` | 2,092 | 2,112 | 1,519 | 2,352 |
| `Nintendo_-_Super_Nintendo_Entertainment_System` | 3,664 | 3,748 | 3,683 | 3,758 |
| `Nintendo_-_Game_Boy` | 2,249 | 1,671 | 1,502 | 1,647 |
| `Sega_-_Saturn` | 2,335 | 2,270 | 1,839 | 2,294 |
| `Nintendo_-_Nintendo_3DS` | 1,967 | 2,009 | 1,541 | 2,148 |

Sample fetches:

- `Sonic The Hedgehog 3 (USA)` title and snap: **320x224**, native MD.
- `Super Metroid (Japan, USA) (En,Ja)`: **256x224**.
- `Tetris (World) (Rev 1)`: **160x144**, 2-bit.
- Saturn is stored rescaled to 512 wide, not at native resolution: four
  titles/snaps were 512x326–398.
- 3DS snaps are 400x480 (both screens stacked) or about 394x218.
- The 3DS `Kid Icarus - Uprising` logo: 400x175 RGBA.

Gotchas:

- Some entries are git **symlinks**. For those, raw.githubusercontent
  returns the target file name as 51 bytes of text, and the page has to
  follow it.
- Names are exact No-Intro strings, so a lookup needs a fuzzy match against
  the tree listing. The core's `normalize_title` fits: lowercase
  alphanumerics, with the `(Region)` tags dropped.
- Unauthenticated api.github.com allows 60 requests an hour per IP. One tree
  per system, cached, is well within that.
- The repos carry no licence file. The images are the publishers' art,
  curated by fans for RetroArch.

**Wikimedia Commons / Wikipedia.** This has a keyless API with CORS
(`&origin=*`) and a search (`generator=search&gsrnamespace=6`) that returns
thumbnail URLs and each file's licence (`extmetadata.LicenseShortName`). It
is good for console and company logos: SEGA, SNES, the Saturn JP/US logos,
the Mega Drive JP logo and the Nintendo 3DS logo are all "Public domain"
(PD-textlogo). Thumbnails now come from `thumb.wikimedia.org` in fixed
steps (a request for 640 px returned a 960 px thumb). Game box art and
screenshots on Wikipedia are non-free, mostly low resolution, and hosted
under enwiki fair-use rules. They are reachable, but libretro has better
images. Wikimedia's policy asks API clients to send a descriptive
User-Agent, which a browser cannot set. The `Api-User-Agent` header is the
documented substitute, and it costs a CORS preflight.

**OpenGameArt.** CC0/CC-BY/OGA-BY assets: good for generic pixel art and
fonts, useless for specific games. It has no API and no CORS, so the only
routes are download and drop, or a proxy.

**Lospec.** The palette API (`/palette-list/<slug>.json`) has CORS and is
useful for "snap to a known palette" (Game Boy DMG green, PICO-8, …).
Gallery images are not readable.

**The Spriters Resource / Sprite Database.** This is the richest sprite-sheet
collection, but its CDN has no CORS. Its terms invite personal and
non-commercial reuse, forbid commercial use, and ask for credit. Route: open
the sheet in a tab, save it or drag it, then drop or paste it into the
Studio.

**Tenor / Giphy.** Both APIs have CORS and need a key: Giphy's public beta key
is retired (403), and Tenor's demo key is dead (400 `API_KEY_INVALID`).
Giphy's media CDN is readable without a key. Giphy's terms require visible
"Powered by GIPHY" attribution. Giphy has real pixel-art Sonic sprites
(giphy.com/explore/sonic-pixel), but the results are mixed: fan art, anime
and video clips. These GIFs are full-motion and need the same framing and
palette work, so they are a nice-to-have behind a key field.

**IGDB / TheGamesDB / ScreenScraper / RAWG.** All need accounts.

- **IGDB** needs a Twitch client id plus an OAuth client-credentials token,
  allows 4 req/s, and is non-commercial under Twitch's terms. The API has no
  CORS and the images do. The client secret must never go in a page, so this
  one needs the local helper.
- **TheGamesDB** keys are requested on its forum and must not be public. It
  has no CORS anywhere.
- **ScreenScraper** needs a developer id/password from its forum plus the
  user's own account. Its API sends ACAO `*`, so it works from the page once
  the owner has credentials. It is what RetroPie/Batocera scrapers use:
  screenshots, wheels (logos) and marquees. Its per-day quotas are
  community-reported and not verified here.
- **RAWG** has a free key and allows commercial use up to 100k MAU. The API
  has no CORS and `media.rawg.io` does. It is mostly modern-game
  screenshots, the least useful here.

**SteamGridDB.** Its "logos" category (transparent wordmarks) would be ideal
for the logo case. Neither the API nor `cdn2` sends CORS (see also
rommapp/romm#3707), so it needs a Bearer key plus the helper.

**itch.io.** No CORS, and no content API for images. Download and drop.

**GitHub raw in general.** Readable everywhere (`*`), so community sprite
packs on GitHub work directly.

### Escape hatches (no CORS needed)

| route | works over `http://<lan-ip>` | notes |
|---|---|---|
| File picker (`<input type=file>`) | yes | the universal fallback; verified in the Studio prototype |
| Drag-and-drop of a file from the desktop / Finder | yes | verified path in the prototype (`dataTransfer.files`) |
| Drag an `<img>` from another tab | Chromium: gives a File **and** a URL; Firefox: URL only (Mozilla bugs 1437126, 1812611) | not verified with a real cross-tab drag (a synthetic drop only proves the handler). The page falls back to fetching `text/uri-list`, which needs CORS. |
| Paste (`paste` event, `clipboardData.files`) | **yes**, no secure context needed | copy an image in any tab, paste into the Studio. Handler verified with a synthetic `ClipboardEvent` in `studio/drive.py`. |
| `navigator.clipboard.read()` | **no**: secure context only (https or localhost) | fine on the Mac at `localhost:8080`, not on the device |
| Web Share Target (PWA manifest) | **no**: needs https, an installed PWA and a service worker. iOS Safari does not support receiving shares at all. | closed for the device as served |

**A local helper.** The owner already runs `tools/mock-device.py --proxy` on
his Mac. A `GET /fetch?url=…` route there could make any host readable,
using an allowlist of hosts and a size cap. It could also hold API secrets,
which cover IGDB's Twitch secret and the SteamGridDB, TheGamesDB and
ScreenScraper keys. It is the right place for anything keyed, but it is a
second process to run. The ESP32 should **not** proxy: TLS plus image
transfers cost heap and frame time (CLAUDE.md "Heap ceiling"). The public
images.weserv.nl proxy works (ACAO `*`) but sends every URL through a third
party with no SLA. It is acceptable as an opt-in setting, not as a default.

## 2. Emulators in the browser

The emulator agent's prototype is in `experiments/art/emulator/`
(`ejs.html`, `binjgb.html`, `grab.py`, `README.txt`, and `out/` with PNGs
and `*_report.json` per run). All measurements were taken in headless
Chromium with SwiftShader (software WebGL), so a real GPU is at least as
fast. The ROMs live outside the repo; `README.txt` links them:

- **GB/GBC:** µCity v1.3, code GPLv3+ and media CC BY-SA 4.0
  (github.com/AntonioND/ucity).
- **Mega Drive:** Miniplanets REMIX REV04, zlib
  (github.com/sikthehedgehog/miniplanets).
- **Mega Drive:** Lunch Break v1.0, GPL-3.0
  (github.com/GoodPraxis/lunch-break).

**Loading a local ROM with no upload works.** The file picker's `File`
becomes a blob URL, which is handed to the emulator. Frames were grabbed
into PNGs: `gb_gambatte_*`, `md_gpgx_*`, `md_lunch_*`, `gb_binjgb_*`.

### EmulatorJS 4.2.3 (gambatte, mgba, genesis_plus_gx)

| | GB | MD |
|---|---|---|
| downloaded, vendored locally | 1.71 MB (core .data 967 KB, `emulator.min.js` 426 KB, `extract7z.js` 280 KB) | 1.95 MB (core .data 1.20 MB) |
| downloaded from cdn.emulatorjs.org (compressed JS) | 1.18 MB | 1.42 MB |
| file pick → first frame | local 0.77–2.5 s (typically 0.85 s); CDN 2.7–4.3 s | same range |
| unthrottled fast-forward | 1,400–3,000 fps | 500–770 fps |
| save state size / save or load time | 182 KB / about 5 ms | 1.0 MB / about 5 ms |

**Grabbing frames:**

- **The canvas is the wrong source.** It is 640x480 WebGL, the game is
  scaled by a non-integer 3.33x, and it reads back blank without forcing
  `preserveDrawingBuffer`.
- **Hooking the texture upload is the best way.** Hooking
  `tex(Sub)Image2D` catches the core's raw RGB565 framebuffer at native
  size. The copy itself takes 0.04–0.2 ms. It takes 6–18 ms end to end,
  including waiting for the next frame and PNG encoding. MD boots at
  256x192, then switches to 320x224.
- **`gameManager.screenshot()` is native too, but slow:** 55–100 ms per
  grab.

**Running headless:**

- Pause works, and loading the same save state twice gives identical
  frames. So "fast-forward to the title screen, save a state, grab N frames"
  is deterministic.
- A hidden emulator keeps running. A background tab was not tested and will
  be rAF-throttled.
- **Audio blocks emulation.** If the AudioContext stays suspended (autoplay
  policy), emulation stalls after frame 1, because RetroArch syncs to audio.
  Setting `audio_sync = false` in the generated retroarch.cfg fixes it: 60
  fps, muted, on GB and MD. EmulatorJS has no official option for this, so
  the prototype patches `getRetroArchCfg`.
- **Plain http from a LAN IP works for non-threaded cores.** Threaded cores
  need SharedArrayBuffer. That needs COOP/COEP, which is ignored on a
  non-secure origin, so with threads on the core never started. None of the
  cores used here need threads.
- **It phones home.** Even fully vendored, `emulator.js` fetches
  `cdn.emulatorjs.org/stable/data/version.json`.
- **Vendoring:** one build variant per core is about 1 MB. The three-core
  subset with all four variants is 15 MB, and the full release 7z is 290
  MiB.

### binjgb (Game Boy / Game Boy Color only)

Tested hands-on (MIT):

- 102 KB total (88 KB wasm, 14 KB JS).
- 10 ms to first frame and 0.2 ms per grab: RGBA is read straight from wasm
  memory.
- About 2,270 fps unthrottled, with no audio, WebGL or rAF needed.
- Deterministic save states (200 KB).

It is the right shape for OpenDMD: small, licence-compatible, and a pure
function from ROM plus frame count to pixels.

### Other options

- **Nostalgist.js:** an MIT wrapper, about 340 KB; the source was read, not
  run. It loads RetroArch cores from jsDelivr and has `screenshot`,
  `saveState`/`loadState`, `pause` and direct retroarch.cfg. A cleaner API
  than EmulatorJS, with the same core licences and CDN dependency.
- **SNES:** snes9x is on the EmulatorJS CDN (non-commercial licence). bsnes
  is not there.
- **wasmboy:** GPL-3.0, last pushed 2023.
- **jsnes:** Apache-2.0, NES only.
- **Saturn:** yabause is on the EmulatorJS CDN (991 KB core, HLE BIOS
  option). It is untested and has no browser performance data; assume slow
  and treat it as an experiment.
- **3DS:** nothing viable. Citra is gone, and the only thing found is a
  hobby Azahar wasm port (about 10 MB).

### Licensing

Checked from the licence texts in the repos and each core bundle's
`licence.txt`:

- EmulatorJS and RetroArch: GPL-3.0.
- gambatte: GPLv2.
- mgba: MPL-2.0.
- **genesis_plus_gx: may not be sold or used in a commercial product.**
- **snes9x: non-commercial.**
- bsnes: GPLv3.
- beetle-saturn: GPL-2.0.

Every EmulatorJS core is RetroArch plus the core in one wasm file, so each
bundle is GPLv3 combined with the core's own terms. For Genesis Plus GX and
snes9x those terms conflict (the agent's reading, not legal advice).

- **(a) Shipping cores in this MIT/Apache repo or on the card image:** no.
  It brings GPL source obligations and non-commercial terms into the
  project. binjgb (MIT) is the exception.
- **(b) The page loading cores from a CDN when used:** OpenDMD distributes
  nothing, so the app stays MIT. The cost is an internet dependency on a
  third-party CDN (cdn.emulatorjs.org, or jsDelivr for Nostalgist) that can
  change or disappear.
- **(c) The user drops core files on the card:** the cleanest legally, but
  it adds friction.

### Verdict

This is worth doing later, not first.

1. Ship binjgb for GB/GBC in the Studio's Import step: "From a game file".
2. MD and SNES get an opt-in "load emulator from the internet" core (EJS or
   Nostalgist), with `audio_sync=false` and no threads. Frames come from the
   texture-upload hook, never the canvas.
3. The UX: fast-forward to a chosen second, show a scrubbable strip of
   frames, and let the user pick frames or a range. An emulator run gives
   *animations*, such as attract-mode loops, that no thumbnail site has.

The processing still matters. On the Miniplanets title screen
(`process/out/f_emulator_title.png`), the auto 1:1 window lands on the
"PASSWORD / OPTIONS" menu text, because text is the busiest thing there. The
auto 256x64 window plus mode 2:1 lands on the wordmark, and it is crisper
than area 2:1. The Studio should try both scales and prefer the one whose
window holds more distinct colours, and the user always gets the final
drag.

## 3. Processing that looks good on 128x32

The prototypes are in `experiments/art/process/`: `art.py` has the
primitives and `demos.py` the five cases. Every sheet is rendered through
`led.py`, a numpy port of `opendmd-render`'s `LedRenderer`. `check_led.py`
renders a test pattern through both it and the real wasm `Preview` from
`www/pkg` at scale 8: **mean and max difference 0**
(`process/out/check_led.png`), so the sheets show what the web preview
shows.

What the pack does, measured on a 600-file random sample and the scroll
entries:

- Every file is 128x32. Median 90 frames (p10 21, p90 362).
- Delays: 30 ms is by far the most common, then 20, 40, 50 and 100 ms.
  Almost all files loop forever (`loop=0`).
- Frame 0 has a median of 10 colours (p90 42).
- `*_SCROLL_*` files move **1 px per frame**, occasionally 2, at 30–50 ms.
  Several hold 1.5 s on the first or last frame.
- Looking at a contact sheet of MD/SNES/GB entries, almost all of the art is
  **native pixels at 1:1**: a sprite or a strip of a scene, often on black.
  A few are 2:1.

### (a) A 256x224 screenshot → `a_screenshot_resample.png`, `a_screenshot_quantise.png`, `a_source_windows.png`

The source is a CC0 Kenney "Pixel Platformer" scene, recovered to native
pixels from its 2x sample image and cropped to 256x224.

- **Recover native pixels first.** `native()` looks at where the strong
  colour edges fall (|ΔRGB| > 60, so JPEG ringing is ignored) modulo k, for
  k = 8…2. It accepts the largest k with more than 80% of edge weight on one
  phase, then samples block centres.
  - A 2x or 3x nearest-neighbour copy is recovered exactly.
  - A 2x copy saved as JPEG q85 gets the right size, with JPEG noise in the
    colours. The fix is to snap to the palette afterwards.
  - A 2.5x bilinear copy is correctly refused (k=1). Those images get the
    smooth path.
- **Pick the window by interest.** Interest is the luma gradient plus "not
  the dominant colour", summed over an integral image, with a mild pull to
  the centre. It takes about 5 ms in numpy for a 256x224 frame, and is a
  plain O(w·h) loop in JS. Always let the user drag the window: "busiest" is
  often right, but a title screen's logo wins against a scene's ground.
- **Resampling, side by side:**
  - *Fit the whole screen* gives 36x32 of mush: never do this.
  - *Nearest 2:1* drops every other row, so outlines break.
  - *Area 2:1* invents 138 colours of blends, and outlines blur into the
    fill.
  - *Mode 2:1* (the most common colour per 2x2 block, ties to the colour
    furthest from the block mean) invents no colours (19, all from the
    source) and keeps 1 px outlines. It is the right 2:1.
  - **1:1 native crop is the crispest and matches the pack.** Make it the
    default for anything that detects as native pixel art.
- **Background keying.** The pale sky is most of the panel's light, so it
  glares. "Background → black" keys the commonest colour **inside the
  window**. Keying the image's commonest colour picked a sky tone the window
  never showed; the Studio prototype hit this and was fixed. The result
  reads like the pack's sprites on black
  (`studio/out/file_native_window_keyed.png`).
- **Quantising to 16, dithering off:**
  - Median cut and NeuQuant (`color_quant` 1.1.0 via `process/nq/`, the
    exact quantiser the ODM encoder uses past 256 colours) both **drop the
    yellow coin and the blue gem**. Those are rare pixels, and count-driven
    quantisers sacrifice them.
  - Floyd–Steinberg adds sparkle noise on the flat fills, which LEDs
    exaggerate.
  - **Spread pick** first snaps to the source's own colours, then takes the
    most-used colour, then repeatedly the colour furthest from everything
    picked, weighted by log(uses). It keeps both accents. It is about 40
    lines, in `art.q_native` and `studio/pipeline.js` `buildPalette`.

### (b) A sprite sheet → `b_spritesheet.png`, `b_segments.png`, `b_pan.gif`, `b_walk.gif`

- **Pan:** a 128x32 window over the sheet's busiest 32-row band, 1 px per
  40 ms, one way, with 1 s holds. It works, but a raw tile sheet panned is
  "a sheet", not "a scene". It is more useful for wide screenshots and
  pre-assembled strips.
- **Walk cycles by segmentation:** 8-connected blobs of opaque pixels, then
  consecutive blobs are grouped when their colour histograms overlap by more
  than 0.75 and their sizes are within 4 px.
  - On the spaced sheet (1 px gutters) this gives 27 sprites → 14 cycles in
    3 ms, and the pairs and triples are the real animations
    (`b_segments.png`).
  - On the **packed** sheet the sprites touch and you get 22 blobs for 27
    sprites. A real sheet needs a grid fallback: detect a repeating cell
    pitch, or let the user draw one cell. Rips from The Spriters Resource
    usually have a flat background colour instead of alpha, so "transparent"
    means "the border colour".
- The walk demo puts four cycles walking across the panel, bottom-aligned,
  1 px per 40 ms, advancing the cycle every 8 frames. This is the most
  pack-like output of the five.

### (c) A logo → `c_logos.png`

The inputs are the SEGA, SNES, Mega Drive JP, Saturn JP and Nintendo 3DS
logos, all public domain on Commons.

- **Fit:** trim the transparent border, fit to 30 px high (a 1 px margin),
  or to the width if that is tighter. For a very long wordmark (width at 32
  px high > 192), pan it instead.
- **Resample:** area/box on premultiplied RGBA. This is a smooth vector
  render, not pixel art, so area averaging is right here.
- **Lift dark strokes.** Black or grey wordmarks vanish on a black panel
  (Saturn, 3DS). At full resolution, before resampling, flip the lightness
  of dark, unsaturated, opaque pixels, unless they cover more than 60% of
  the logo's box. The Mega Drive logo's black backing square correctly stays
  black, and its red "M" and green "D" stand on their own.
- **Palette at 16 or fewer.** Median cut is fine for simple two-colour
  marks. **NeuQuant loses the green "D" entirely** on the Mega Drive logo
  and greys out the Saturn planet. Spread-pick at 8 keeps every hue. Thin
  lettering under about 5 px tall is noise either way (the "MEGA DRIVE" under
  the MD emblem). The Studio should warn, or suggest cropping to the emblem.

### (d) A tall image → vertical scroll → `d_scroll.png`, `d_source_column.png`, `d_scroll.gif`

- A 430-row native image. The busiest 128-px column scrolls top to bottom,
  1 px per 40 ms, with 1.5 s holds at both ends as in the pack: 399 frames,
  about 19 s, 29 colours.
- For inputs wider than 128 at native scale: pick the column (as here), or
  do mode 2:1 first if the whole width matters.
- Frames are complete, one global palette. The prepared copy was 198 KB and
  the GIF 520 KB. The GIF writer used here (Pillow) doesn't diff frames. A
  diffing encoder shrinks the GIF but not the prepared copy, which is what
  plays.

### (e) Title text when there is no art → `e_title_cards.png`, `e_*.gif`

- **Fonts:**
  - **Spleen 8x16** (BSD-2-Clause, github.com/fcambus/spleen) for the title.
  - **Tom Thumb 3x5** (MIT, and also CC0/CC-BY 3.0 per the author,
    robey.lag.net/2010/01/23/tiny-monospace-font.html) for the platform
    tag.
  - Both are BDF, parsed by `process/font.py` (60 lines).
- **Layout:** the tag is top-left in the platform's accent colour and the
  title is centred, white, with a 1 px shadow. The background is a
  platform-coloured vertical gradient banded to 8 steps, so it uses a few
  colours rather than hundreds.
- Up to 15 characters fit on one line with the tag. Longer titles use the
  **most balanced** two-line split with no tag ("Streets / of Rage 2", "Kid
  Icarus / Uprising"); a greedy wrap left a lone "2". Anything longer
  scrolls as a marquee at 1 px per 30 ms.
- A still card is a single frame with a 5 s delay: about 1.4 KB, and a
  prepared copy under 1 KB.

### Quantisation and timing defaults

| setting | default | why |
|---|---|---|
| palette | exact if ≤ 256 colours across the whole animation, else spread-pick | no invented colours; the ODM copy keeps exactly what the GIF has (`quantised: false`) |
| colour cap offered | 256 / 16 / 8 | 16 is the "pack look" (median frame-0 colour count is 10) |
| dithering | off | LEDs turn dither into sparkle |
| still | 1 frame, 5,000 ms | the engine's dwell handles how long it stays |
| pan / scroll | 1 px per 40 ms (25 px/s); 30 ms and 50 ms offered | pack scrolls: 1 px at 30–50 ms |
| holds | 1,500 ms on the first and last frame | as in the pack's scrolls |
| sprite cycles | advance every 8 frames (320 ms) at 40 ms | reads as walking at 25 px/s |
| delays | ≥ 20 ms, never 0 | the player floors at 20 ms and turns 0 into 100 ms (`docs/odm-format.md`) |

### Everything the prototypes made plays as-is

The five demo GIFs were filed as `gifs/Consoles/*.gif` in a scratch card and
run through `opendmd-pack convert --layout bucketed --verify`. Result:

- 5 of 5 identical, 1,454 frames compared.
- 0 files needing quantisation. Palettes run from 9 to 87 colours.

The Studio page runs `encodeGif` on every GIF it makes and reported
`quantised: false` every time.

## 4. The Studio (UX concept)

**Where it lives.** It is a sheet opened from Library ("Make art…" next to
"Add GIFs…", device only), not a fifth nav item. It is a way to add GIFs,
and the user's concept stays "GIFs and playlists". There are five steps on
one screen, top to bottom on a phone, as panels on a desktop.
`studio/index.html` is a working sketch of this layout
(`studio/out/studio_page.png`).

1. **Import.** Choose one:
   - *Find for a game*: platform plus title, prefilled from RePlayOS
     context when it is following a game. It searches libretro-thumbnails
     (Titles / Snaps / Logos) and Commons, and shows a strip of thumbnails.
   - *Paste*: listens for ⌘V and says so.
   - *File* and *Drop*.
   - *URL*: on failure it says "this site doesn't let pages read its images.
     Save it and drop it here, or copy and paste it". It never mentions
     CORS.
   - *From a game file* (emulator), later, behind the emulator decision.
   - Keyed sources (Giphy, SteamGridDB, ScreenScraper, IGDB) appear only
     once a key is set in Settings.
2. **Frame.** A zoomed source with the 128x32 window drawn on it: drag to
   move it, and use Auto to re-pick the busiest spot.
   - *Fit*: Pixels (1:1, default when native pixels are detected), Pixels
     ½, and Fit (smooth; logos and photos).
   - *Motion*: Still, Pan →, Scroll ↓, and later Walk (sprite cycles found
     by segmentation, shown as chips to pick from).
   - Motion is auto-chosen from the shape: wide and short pans, tall and
     narrow scrolls.
3. **Look.** Colours (Auto / 16 / 8), "Background → black", brightness (a
   gain applied to the pixels, since the panel's own brightness is a device
   setting), and speed (slow / normal / fast = 50 / 40 / 30 ms).
4. **Preview.** The existing `PanelPreview` (wasm `Preview`) plays the GIF
   being made. It shows frame count and length, not bytes.
5. **Save.** Category (default `Consoles`), platform (a chip row from the
   known list, prefilled from RePlayOS `system` → tag, e.g. `sega_st` →
   `SATURN`), title (prefilled from `game_name` with the region tags
   removed), and the next free variant `NN`.
   - The filename is shown as a caption, not as an editable path.
   - Save uses the Add GIFs path: PUT the GIF, prepare its copy, then offer
     to add it to the playlist in play.
   - "Send to panel" plays it at once.

**What to hide (the product rule).** Hide `.odm`, "prepared copy",
buckets, palettes as numbers, CORS, API endpoints, No-Intro names, and GIF
byte sizes (a scroll is 0.5 MB and nobody should care). Show "colours", the
panel preview, and the filename the Library will list. Keys go under
Settings → "Art sources", one field per service with a link to where to get
it. The flag vocabulary goes unexplained.

**The filename grammar bites.** When the prototype saved "Sega Saturn logo",
the name `SATURN_Sega_Saturn_Logo_01.gif` parsed back as title **"Sega
Saturn"** + flag **`Logo`**, because `LOGO` is a keyword flag. That happens
to be right (the pack has `MD_LOGO_Sega31`), but it shows that Save must
round-trip the name through `parsePath` and show the parsed title. Save
should map a "Logo" choice to the flag deliberately, not by accident. The
same applies to titles ending in digits: "Streets of Rage 2" must stay a
title and not become a variant. The `_NN` suffix guarantees that, because
the last token is the variant.

**Effort, by stage** (one developer who knows this codebase, including
smoke-test coverage in `tools/web-smoke.py`):

| stage | work | estimate |
|---|---|---|
| Import: file, drop, paste, URL | reading to ImageData; error copy | 0.5 d |
| Import: libretro-thumbnails lookup | per-system tree listing (cached), fuzzy match via `normalizeTitle`, symlink follow, thumbnail strip | 1 d |
| Import: Commons search | API + licence display | 0.5 d |
| Frame | native detect, auto window, drag, fit modes, still/pan/scroll | 1–1.5 d |
| Look + palette + GIF encode | spread-pick, key-to-black, lift, speed; encoder in wasm (`gif` crate) | 1 d |
| Save | grammar name, `parsePath` round trip, next-free `NN`, Add GIFs reuse | 0.5 d |
| Walk cycles (segmentation + grid fallback) | later | 1.5 d |
| Keyed sources + Settings "Art sources" | Giphy/ScreenScraper direct; IGDB/SteamGridDB via the Mac helper | 2 d + helper 1 d |
| Emulator import | binjgb GB/GBC (vendored, 102 KB) with frame strip + range pick: 2 d; optional CDN cores for MD/SNES: +2 d | 2–4 d |

**Where the code goes.** The pixel work (native detect, window scoring, mode
downscale, palette pick, GIF encode) should be Rust in `opendmd-web`, with
no web-sys, tested natively like `player.rs`. The alternative doesn't fit:

- The hand-written JS + CSS budget is 80,669 of 81,920 B. The Studio's
  view code alone will not fit, so it must be a lazily imported module with
  its own budget line, or the budget has to move.
- The wasm budget has 45 KB of headroom (205 of 250 KB). Adding the `gif`
  crate's **encoder** to a module that already decodes costs **+15.2 KB**
  of wasm, measured on a scratch crate before `wasm-opt`. The primitives
  are small.
- `gifenc` (MIT, 3.9 KB gzipped JS) is the alternative the prototype uses.
  It works, but it would be a second GIF implementation next to the core's.

## 5. Rights and etiquette

The owner's stated scope is personal use in his home, for games he owns.
Per source:

| source | key | terms that matter here |
|---|---|---|
| libretro-thumbnails (GitHub) | none | no licence file; publisher art curated for RetroArch; GitHub raw/API rate limits (60 unauthenticated API calls/h) |
| Wikimedia Commons / Wikipedia | none | per-file licence in the API response (show it); PD-textlogo logos are free; enwiki box art is non-free fair use; send `Api-User-Agent` |
| OpenGameArt | none | per-asset CC0 / CC-BY / OGA-BY / GPL; attribution where required |
| Lospec palettes | optional (free) | palettes are free to use |
| The Spriters Resource | none | reuse OK non-commercially with credit; no commercial use; rips remain the publishers' |
| Giphy | **yes** | "Powered by GIPHY" attribution; key must be per app |
| Tenor | **yes** (Google Cloud) | 1 req/s default; v1 shut down in 2026, use v2 |
| IGDB | **yes** (Twitch client id + secret) | non-commercial per Twitch; 4 req/s; secret cannot live in a page, so helper |
| ScreenScraper | **yes** (dev id + user account) | per-user daily quotas (community-reported, unverified) |
| TheGamesDB | **yes** (forum) | key must not be public, so helper |
| RAWG | **yes** (free) | free up to 100k MAU (rawg.io/tos_api); check its attribution clause before use |
| SteamGridDB | **yes** (free Bearer) | API terms; no CORS, so helper |
| images.weserv.nl | none | third-party proxy, best-effort; opt-in only |

This means the app needs a Settings → "Art sources" card with one key field
per keyed service. Keys that are client secrets (IGDB, TheGamesDB) belong
only in the Mac helper's environment, never on the card. Automated fetching
should be user-initiated, one request at a time, with listings cached. No
source here invites bulk scraping.

## Recommended first milestone

**"Make art" v1: native pixels from libretro-thumbnails, a file or a paste →
a still, pan or scroll GIF → saved and prepared.** No keys, no proxy, no
emulator.

1. `opendmd-web/src/art.rs` (no web-sys, native tests):
   - `native_scale`, `best_window`, `down_mode`
   - `spread_palette`, `key_background`, `lift_dark`
   - `encode_animation(frames, delays, max_colours) -> gif bytes`, using the
     `gif` crate's encoder and one global palette
   - exported as `art*` functions
2. `www/js/studio.js`, lazily imported from Library, with its own size
   budget line:
   - Import: file, drop, paste, URL
   - "Find for a game", which searches `libretro-thumbnails/<System>` trees
     through api.github.com and loads from raw.githubusercontent.com, with
     platform → repo for MD, SNES, GB, GBC, GBA, SATURN, 3DS, NES, PCE, SMS
     and GG
   - Frame and Look as in the prototype
   - the existing `PanelPreview`
   - Save through `prepareBytes` and the Add GIFs playlist offer
3. `tools/web-smoke.py`: a Studio pass against the mock. It routes a
   libretro raw URL to a generated PNG (no network in CI), then checks:
   - the saved name parses to the chosen platform and title
   - the GIF lands in `/gifs/Consoles/`
   - its prepared copy is byte-identical to `opendmd-pack`'s
   - `quantised` is false
   - no "odm" or "CORS" text appears in the view

This covers MD, SNES and GB title screens and in-game snaps at native
resolution, which are the owner's gaps. Saturn and 3DS thumbnails are not
native pixels (Saturn is rescaled to 512 wide; 3DS snaps are 400x480
stacked screens), so they will mostly use Fit plus the Logos folder. Follow-ups, in order of value:

- RePlayOS prefill (once "Follow RePlayOS" lands)
- walk cycles
- the keyed sources through the Mac helper
- the emulator import

### Addendum: pixilart.com (probed 2026-09-23)

The owner noticed the gallery API (`/api/w/gallery/<page>/<id>/popular?...`)
answers with `access-control-allow-origin: *` in his browser. From anything
that is not his browser it does not answer at all: curl, Playwright's
request context and a headless page all get **HTTP 403 with
`cf-mitigated: challenge`** (a Cloudflare managed challenge, "Just a
moment…"), no CORS header on the 403, so `fetch()` fails. The clearance is a
cookie scoped to pixilart.com, which a cross-origin `fetch` from the panel's
page cannot send under a `*` policy, so the API is most likely unreachable
from the app even in a real browser — to be confirmed by running, from the
OpenDMD page's console:

```js
fetch('https://www.pixilart.com/api/w/gallery/10/33478920/popular').then(r => r.status).catch(e => e.message)
```

Result: the owner got **200** from the app's origin — but only because his
browser had already passed the challenge on pixilart.com. A first-time user
would have to visit pixilart in the same browser first, and the clearance
expires. **Decision (owner, 2026-09-23): filed as potential, not included** —
no source that needs extra hoops. Pixilart works today as a paste /
drag-and-drop source, which is how it should appear in the Studio.
