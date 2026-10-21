# The web app

The management UI is a **WebAssembly page the player serves out of its own
flash** at `http://opendmd.local/`. All the heavy work happens in the browser —
parsing and filtering manifests, previewing files with the panel look,
preparing GIFs for playback — with the same `opendmd-core` and `opendmd-render`
Rust the firmware and the simulator run. The ESP32 is a file server plus the
small JSON API in [`api.md`](api.md). No framework and no bundler: hand-written
HTML, CSS and ES modules in `opendmd-web/www/`, plus the `opendmd-web` wasm
module.

```bash
make web                                   # wasm-pack build + a .gz beside every file + the www pack + the size budget
make web-mock ROOT=~/scratch/dmdos-card # a fake device on :8080 (OVERLAY=dir keeps the card copy untouched)
make web-mock PROXY=http://192.168.1.69    # this build, same-origin against a real device
make web-serve                             # plain static server: local mode
make web-smoke                             # headless Chromium through every view (DEVICE=http://… adds a real-device pass)
make www-push HOST=opendmd.local           # write the pack into the player's flash over WiFi
```

## Device mode and local mode

**The API.** Every route the player serves, with its parameters, replies,
limits and a `curl` example, is in [`docs/api.md`](api.md). The same
reference is in the web app (Settings → Advanced → About → API reference,
`http://opendmd.local/api.html`), with the examples already pointing at
that player.

**Device mode or local mode.** The page asks `GET /api/status`; if a player
answers, it is in device mode and everything below talks to that device. If
not (a laptop running `make web-serve`), it offers a folder picker: point it
at an extracted copy of a card and the same views work over the picked files,
read one at a time as they are needed. Writes in local mode go to a folder you
grant (Chromium's File System Access API) or become downloads.

**Users see GIFs and playlists.** A GIF's *prepared copy* ([`card.md`](card.md#prepared-copies)) is a cache: the player plays it when it
exists, previews fetch it first and fall back to the GIF, and the UI never
names the format or shows a copy's path.

## The views

**The views** — a left rail on a laptop, a bottom bar on a phone:

- **Now** — what the panel is playing (title, platform and category parsed
  from the path), polled every 2 s, with skip, pause, a brightness slider and
  **Send to panel** (any card path, `POST /api/play`: it plays now and the
  schedule carries on), and the device's health: uptime, address, free heap and its sampled floor,
  refresh rate, deadline misses and worst lateness. A **panel-look preview**
  of the same file plays alongside — fetched once when the file changes and
  played in the browser, so it is *not frame-synced* with the panel. It
  fetches the file's prepared copy first (cheap for the device; one 404 when
  there is none) and can be switched off. The LED-look controls (gain,
  bloom, gamma, flat) are here too and are remembered per browser. In local
  mode this view is the simulator: a manifest or the whole folder, a platform
  filter, shuffle and a seed, with the firmware's 3-30 s dwell.
- **Library** — the GIFs on the player, and only those (finding new art is
  **Create Art**'s). One row across the top: a search field (titles, normalised
  the way the core matches them; when nothing matches, a link searches the
  web for the same words in Create Art) and **Add GIFs**; under it the
  categories as tabs (the card's folders under `gifs/`, then *Live* and
  *Effects*) and the platform chips. The list (virtualised: thousands of
  rows) shows each GIF's title, platform and favourite/hidden marks. Pick one
  and the **Details** panel (the right-hand column; a sheet from the bottom
  on a phone) plays it with the panel look and shows its title, platform,
  category, author and file name, with **Send to panel**, **Add to
  playlist…**, **Edit in Create Art**, **Favourite** and **Hide** (in the
  playlist chosen beside them), Smooth motion where it applies, **Edit
  details…** and **Delete…**. Select several rows to add them to a playlist
  together.

  **A GIF's details are its file name** —
  `/gifs/<Category>/<PLATFORM>_<Title_Words>_<NN>[_<author>].gif`, which
  every part of OpenDMD reads the same way ([`card.md`](card.md#tag-grammar)). **Add
  GIFs** (device) asks for each picked file's title, platform, category (or a
  new one) and author, guessing the title from the file's own name (none for
  a name like `QT6JDNGFS3Q4OO6GBKO4LBW2ZZ4MO375.gif`, which must then be
  given one) and showing the name it will get as you type — the number is
  the next free one for that platform and title, so a second Super Metroid
  is `_02`. It uploads each under that name, prepares it in the browser (the
  GIF, then its copy, one file at a time) and offers to add them to the
  playlist in play. **Edit details** changes the same fields, which renames
  the GIF on the card (`POST /api/files/move`: its prepared copy and its
  Smooth motion setting follow) and in every playlist and favourite that
  names it. **Delete** asks first, then removes the GIF, its copy and every
  playlist line and favourite; a category left empty goes too.
- **Playlists** (`#sets`) — the card's own (`filelist.txt` as **All files**,
  the default; `filelist.hall` **Halloween**; `filelist.xmas` **Christmas**;
  any other `filelist.*` by file name; an obsolete `filelist.odm.*` never)
  and `/sets/*.txt`. Open one to edit it; build a new one with the core's
  `Filter` (platforms, categories, title words, dropped flags, shuffle with a
  seed, a limit) over any playlist, or from the Library selection; reorder,
  remove, save (`PUT /api/sets/<name>`), activate (`PUT /api/config {"set":
  …}` — `""` for All files, a root manifest's file name, or a set's name —
  which rebuilds the device's index in a few seconds) and delete. Deleting
  the active playlist switches the player back to All files first. Switching
  is disabled while status says an index is `building`. Entries are always
  GIF paths.
- **Settings** — WiFi (with a scan) and hostname, which need a reboot;
  brightness, dwell, shuffle, platform filter and playlist, which apply
  live (not while an index is building); a **Maximum brightness** under
  Power (default 144, below the level tested safe on the supplied power supply;
  raising it shows a warning, because the panel can reset above it); the **Playback cache** card ("9,826
  of 10,133 files prepared", from `GET /api/cache`) with **Prepare remaining
  files**, which works out what the playlist in play is missing
  (`GET /api/cache/list`) and encodes and uploads it in the background, one
  file at a time, with progress, an ETA and a stop button (starting again
  carries on); rebuild the index and reboot; the firmware version, flash
  variant, panel facts, `index_state` and the web app's own version
  (`www_version`, from the pack's header). Firmware updates go over WiFi
  ([`deploying.md`](deploying.md)).

## Create Art

**Create Art.** A view of its own in the navigation: find a picture or an
animated GIF — on the web or of your own — and turn it into a GIF for the
panel, in six steps. Until something is loaded only step 1 shows; then it
folds to one line (**Change picture** opens it again, the search as it was)
and the other steps appear. **Edit in Create Art** on a Library GIF
opens it here, every frame of it, with its title, platform, category and
author filled in, to **Replace the original** (the same file, so every
playlist keeps it; a changed title renames it, and playlists follow) or
**Save as new**.

1. **Find** — three sources. **Game art**: pick a platform (MD, SNES, GB,
   GBC, GBA, Saturn, DS, NES, SMS, PC Engine, PS1, N64, Arcade), type a
   title, and the title screens, in-game shots, boxes and logos (whichever
   exist) of the best-matching games load from the
   [libretro-thumbnails](https://github.com/libretro-thumbnails)
   collections. **GifCities**: the Internet Archive's search over the GIFs
   of GeoCities, a grid of the GIFs themselves, moving, each with its size,
   narrowed without asking again — **Wide (suits the panel)** (at least
   twice as wide as tall), **Square-ish** or **Any**, **At least 48 px
   wide** (on by default), best match or biggest first, with a count ("37
   of 300"); results are unfiltered, and a loaded one links to the archived
   page it was on (**Seen on…**). **Your own**: choose a file or give its
   address (most sites don't let a page read their pictures; then the
   Studio says so: save it and drop it). Dropping or pasting a picture
   (Ctrl/⌘-V; copy an image in any tab) works whichever source is showing.
   **Click a result to load it** — with a title to save it under, and for
   game art its platform. Nothing is added to the card until it is saved.
   A GIF with two or more frames comes in as an animation
   (with a progress line while its frames are read). A very big one is
   worked on at a smaller size, and the Studio says so.
2. **Frame** — the picture (an animation's first frame) zoomed with the
   128x32 window on it: drag it, tap to move it, **Auto** to re-pick the
   busiest spot. **Pixels** uses the picture's own pixels 1:1 (a 2x or 3x
   enlarged screenshot or GIF is detected and undone), **Pixels ½** halves
   it keeping only colours it has, **Fit** scales a logo or photo to the
   panel's height. A still picture can **Pan →** or **Scroll ↓**, at 50 /
   40 / 30 ms a pixel, with a pause at the ends; an animation keeps its own
   timing.
3. **Look** — colours (auto, or 256 / 64 / 16, picked so that a small
   bright detail survives), background to black, brighten dark outlines
   (black lettering vanishes on the panel), brightness. No dithering. For
   an animation, every frame.
4. **Text** — a line of text in the panel's small or normal font, any
   colour, at one of nine places (plus a nudge in pixels), on every frame
   or on a range of an animation's frames.
5. **Preview** — the panel look, playing. For an animation: play/pause, a
   scrubber, and the **timeline**: a strip of the frames (click one to see
   it; Shift or Ctrl to pick several), **Start here** / **End here** (or
   drag the handles) to trim, **Delete frame** (or the Delete key) and
   **Undo** (Ctrl/⌘-Z), speed ¼× to 4×, **Reverse**, **Ping-pong** (there
   and back), and how many times it plays (forever, or 1 to 10). The frame
   count and the length update as you go. On a device, **Smooth motion**
   (below) is a switch here too: it looks at the animation as it will be
   saved, says how many frames it adds ("Adds 32 frames across 16 moves")
   and the preview plays it smoothed; if nothing moves in steps it can
   smooth (a speed of 2× can be too fast to smooth), it says so and looks
   again after your next change. Save keeps the choice.
6. **Save** — title, platform, category, author and "it is a logo", as
   Add GIFs asks for them: the file name is built from them (the number is
   the next free one) and the Studio shows what the Library will list it
   as, or why it can't be named that way. On a device it goes into the
   chosen category, prepared, and the Studio offers to add it to the
   playlist in play; in local mode it downloads.

The pixel work and the GIF writer are Rust (`opendmd-art-web`), in a second
wasm module loaded only when the Studio opens. The game name lists are on
the card (`www/art/<PLATFORM>.json`, one fetched per search), built by
`uv run tools/art-index.py` from the repositories' file lists; re-run it to
pick up new games. The pictures themselves come from GitHub when you pick
a game, so that part needs the internet. The investigation behind all this
is `docs/art-pipeline.md`.

## Smooth motion

**Smooth motion.** Some GIFs move in steps of 2 px or more a frame — a
marquee, a credits scroll, a sprite crossing the screen — and look jerky on
a panel that could show every pixel of the way. Preview such a GIF in the
Library (on a device) and a **Smooth motion** switch appears under the
preview: "96% of this animation moves in steps of 2 px or more. Smooth
motion adds 402 frames." **Preview** plays the smooth version there first;
switching it on makes the panel play it with the in-between frames added —
each one the picture moved by whole pixels, never a blend, the timing
unchanged — and switching it off goes back to the original frames. Your GIF
itself is never changed. The switch only appears where there is something
to smooth (about one GIF in ten on the stock card); Create Art has the same
switch for an animation you make or edit there, and a GIF that is on stays
on when you edit it there and replace it. The choice is kept on
the card in `/sets/.smooth` and survives clearing the playback cache.
`opendmd-pack smooth analyse <card> [--write]` lists (and can switch on)
every GIF worth smoothing, and `opendmd-pack convert` honours the list.

## Favourites, hiding and schedules

**Favourites, hiding and schedules.** Right-click a GIF in the Library or a
playlist (long-press on a phone, or its "…" button) to make it a favourite,
play it less, hide it, play it longer or loop it — per playlist, so a GIF can
be a favourite in one and hidden in another. Settings → Time sets the zone
(the player keeps time over NTP), and Settings → Schedule picks a playlist
and/or brightness for parts of the week ("Weekdays 18:00–23:00 → Consoles")
and a night mode that turns the panel off or dims it; Now shows when either
is in force and can wake the panel for an hour. The saved playlist and
brightness come back when a rule ends.

## Colour, glow, live items and effects

**Colour, glow and the live items.** The player can show everything in one
colour like a classic dot-matrix display — Settings → Playback → *colour*:
full colour (the default), amber (the plasma-DMD orange), red, green, blue or
white; each pixel becomes that colour at its own brightness (Rec. 601
luminance) — and can give the dots a **phosphor glow**, *off* (the default),
*short* or *long*: a dot that goes dark fades out over ~110 ms or ~200 ms
instead of switching off, as on a plasma display. Both are the last step
before the LEDs, applied by the firmware as it writes the panel and by the
same code (`opendmd_core::look`) in the simulator (`--look`, `--glow`) and in
every preview in the web app, so transitions, title cards and everything
else pass through them. Glow is opt-in because it keeps a 12 KB copy of what
the panel shows while it is lit (none while it is dark or paused), which
transitions and the cache builder would otherwise have. Library → *Live*
lists the two things the panel draws itself: a **clock** (the player's own
time, set over the network; four layouts) and a **scrolling text** (type the
message); preview them, send them to the panel, or add them to a playlist
like any GIF — in a playlist they are lines such as `@clock?date=1` or
`@text?msg=Game%20over` (options: `@clock?style=big|small&seconds=1&date=1&h12=1&color=RRGGBB`,
`@text?msg=…&speed=<ms per pixel, 20..1000>&font=big|normal|small&color=RRGGBB&scroll=0`).
Follow RePlayOS can show the clock in the menus instead of its attract
playlist (Settings → Follow RePlayOS → *attract playlist*: The clock).

**Effects.** Library → *Effects* lists four pictures the panel computes
every frame, at 100 frames a second (the panel's refresh rate): a
**tunnel** (the Chrono Trigger time warp: two textures turning in opposite
directions, flying forward, lit by depth and by a light going round, the
palette cycling through it), **rotozoom** (Mode 7: two textures turning
against each other, breathing in and out, with the warp's sine wobble on
rows and columns — or laid out as a floor running to the horizon), a
**starfield** (192 stars as warp streaks, or dots) and a **wireframe** (a
sphere opening into a torus and a flower and back, or a cube, octahedron,
icosahedron, torus or a 4D tesseract, turning and sweeping across the
panel, every line lit by its depth). Each has a few options — palette
(chrono, fire, ice, amber, rainbow), texture, layers, speed, frame rate
and so on — shown beside the preview, and plays, sends to the panel and
goes in a playlist like any GIF, as a line such as
`@fx:tunnel?palette=fire&drift=1` (the options, all optional: `speed=1..4`,
`fps=100|50|25`, `palette=`, `cycle=0|1`; tunnel `texture=checker|rings|
noise&layers=1|2&twist=0..3&drift=1`; rotozoom `texture=…&layers=1|2&
wobble=0..3&zoom=1..4&floor=1`; starfield `density=1..3&warp=0`; wire
`shape=morph|cube|octahedron|icosahedron|torus|tesseract&color=RRGGBB&
shade=0`). An effect's loop — one turn of the tunnel, one pass through the
stars — is what the dwell counts. They cost the player no memory (their
tables are in flash) and 1-3.5 ms a frame. The palettes are stepped and
have a black band on purpose: the panel shows smooth, dim gradients as
bands anyway. Follow RePlayOS can show one in the menus too (`PUT
/api/config {"attract":"fx:tunnel"}`).

## Size budget

**Size budget**, checked by `make web` (`tools/web-size.py`) on the gzipped
sizes, which are what crosses the WiFi: the wasm module plus its generated JS
glue at most 116 KB, the hand-written JS and CSS at most 56 KB; the Studio's
own wasm at most 150 KB and its JS and CSS at most 24 KB, loaded only when it
opens; the Add GIFs / Edit details dialogs at most 8 KB, loaded when first
used. Today that is 115.8 KB, 51.3 KB, 110.0 KB, 9.3 KB and 6.3 KB. The game name
lists (2.4 MB, 527 KB gzipped, one platform fetched at a time) are printed
without a budget. `make web` writes a `<file>.gz` beside each file, and the
player sends those in preference.

## The demo

[opendmd.github.io/demo](https://opendmd.github.io/demo/) is this app with no
player behind it. When nothing answers `GET /api/status` but a
`demo/index.json` sits beside the page, the app loads that card instead of
offering the folder picker: its files are fetched as they are read, writes
stay in the page, and Now starts playing All files by itself (live items
included). **Use your own card…** in the header swaps in a card folder.

The demo card is the panel's own effects, recorded by the simulator, the
CC0 Kenney Pixel Platformer sprites and a playlist of live items
(`tools/demo-card.py`). `make site` builds the whole public site into
`target/site`: the README as the landing page, these docs as HTML, and the
app with the demo card under `demo/` (`tools/site.py`). `make site-serve`
serves it on :8081; `make site-publish` pushes it to the
`opendmd/opendmd.github.io` repository, which GitHub Pages serves.

## The mock device

**The mock device.** `tools/mock-device.py` (a `uv` script, standard library
only) serves the built app plus a fake device over an extracted card copy:
`/api/status` walking the active manifest (the default rule included),
config, sets, play, stat, the playback cache (`/api/cache`, `/list`,
`/forget`), file uploads and deletes, reindex, a canned WiFi
scan, and the card's files — with the firmware's JSON shapes and quirks.
`--overlay dir` sends every write there so the card copy is never modified;
`--strict` drops the mock's few deliberate deviations and adds the
firmware's directory-scan cost to stat and play; `--proxy URL` forwards
everything but the app to a real device.
