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
limits and a `curl` example, is in [`docs/api.md`](api.md), published on
the project site: `https://opendmd.dev/dev/api` (the latest
docs) and, for each release, `dev/api/v<version>.html` as that release
described it. The web app does not carry it: Settings → Device → About →
API reference opens the page for the player's firmware in a new tab (a
release's own page; the latest for a build from source).

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

**The views** — a left rail on a laptop, a bottom bar on a phone, in this
order: Now, Library, Playlists, Create Art, Settings. A view's address is
`#now`, `#library`, `#sets`, `#create` or `#settings`; Settings' tabs and the
Playlists editor have their own (`#settings/play`, `#settings/look`,
`#settings/time`, `#settings/device`; `#sets/edit`), so a link or the back
button lands on them.

- **Now** — what the panel is playing: the title, then its platform,
  category and flags, then where it is in the playlist ("14 of 10,162"),
  each on a line of its own; pushed by the panel while the view is up
  (`GET /api/events`; firmware without it is polled every 500 ms). The
  remote control is **Skip** (first, the primary), **Pause**, **Previous**
  and **Replay**, a grid of four on a phone, and a brightness slider. A
  **panel-look preview** of the same file plays alongside, kept in step with
  the panel by clock. It fetches the file's prepared copy first (cheap for
  the device; one 404 when there is none) and can be switched off (Settings
  → Look → *Mirror the panel on Now*, per browser). The device's figures
  (uptime, memory, late frames) are Settings → Device → Status. The LED-look controls (gain,
  bloom, gamma, flat) are here too and are remembered per browser. In local
  mode this view is the simulator: a manifest or the whole folder, a platform
  filter, shuffle and a seed, with the firmware's 3-30 s dwell.
  **Recently played** (under the preview) and **Previous** / **Replay**
  (beside Skip and Pause): the device keeps the last 16 distinct files the
  panel started, wherever they came from (`opendmd_core::history`, 640 B of
  RAM, gone at a reboot); Now asks for it (`/api/status?recent=1`) only when
  the file changes. Titles as the Library shows them, the one on screen
  marked; a tap plays it again (`POST /api/play`), Replay restarts the
  file on screen, and Previous is `POST /api/skip?back=1`, which walks back
  one entry a press (twice is two back, not a flip between the last two).
  The simulator keeps the same list in the page by the same rules; firmware
  without it hides Previous and the list. **✎ Edit in Create Art** under the
  title, and a ✎ on each Recently played row, open that item where it can be
  changed, as the Library's Edit does: a GIF in Create Art from the GIF
  itself (every frame, never the player's prepared copy), a Formula in the Formulas editor (*Edit in Formulas*: the
  saved or example formula it is, name and comments included, else the
  panel's one-line text of it). The title cards and the start-up show
  are drawn by the panel itself; Now says so, with no button. A ☆ beside the title makes the
  GIF on screen a favourite in the playlist in play (★ when it is; not for
  the panel's own screens); each Recently played row has the "…" overrides menu for
  that playlist.
- **Library** — the GIFs on the player, and only those (finding new art is
  **Create Art**'s). One row across the top: a search field (titles, normalised
  the way the core matches them; when nothing matches, a link searches the
  web for the same words in Create Art), the count, **⊟** (close every
  folder), **Select** and **Add GIFs**; under it the platform chips. The
  list is a **tree of folders** (`js/tree.js`): the card's folders under
  `gifs/` by human name, then the system `_x`
  ones, each with its count (the last one seen, remembered by the browser,
  until the folder is listed again); a folder opens into its GIFs (each
  folder is listed once a session, when first opened, and shared with the
  playlist editor). The rows are virtualised: Consoles' 3,781 GIFs are a
  screenful of elements. `#library/<folder>` opens one folder at the top
  with the others closed. Keys: ↑/↓, Home/End, PgUp/PgDn, → opens a folder
  or steps into it, ← closes it or steps out, Enter opens a folder or a
  GIF's details, Space ticks in Select mode (`role="tree"`, `treeitem`,
  `aria-level`, `aria-expanded`). A search is of every GIF on the card
  (each folder listed once, one at a time, then kept): only the folders
  with matches show, open, with their match counts — the category facets —
  and the platform chips count the matches, most first, and narrow them;
  clearing the box clears the platforms picked with it. Each GIF row shows
  a thumbnail (below), its title, platform and favourite marks. Pick
  one and the **Details** panel (the right-hand column; a sheet from the
  bottom on a phone) plays it with the panel look and shows its title,
  platform, category, author and file name, with **Send to panel**, **Add
  to playlist…**, **Edit in Create Art**, **Favourite** (in the playlist
  chosen beside it), Smooth motion where it applies, **Edit
  details…** and **Delete…**. **Select** (or a long press on a row, on a
  phone) shows a checkbox on every row and folder (a folder's ticks every
  GIF in it that the search shows) and a bar with **Select all shown**,
  **Add to playlist** and **Done**: a tap then picks a row instead of
  opening it. A GIF whose name has no title, or only a number
  (`N64_LOGO_01.gif`), is listed by its platform and flags: "N64 Logo 01".

  **Thumbnails** (`js/thumbs.js`; on by default, Settings → Device →
  *Thumbnails in the Library and playlists*, per browser): the first frame,
  64×16 CSS px, and on hover or keyboard focus (350 ms) a short clip of the
  first frames. Light on the device by design: only
  rows in view ask, at most two requests in flight, none while a list is
  flung; the prepared copy first (`cachePathFor`), the GIF on a 404, read as
  a stream and **cut off once the first image is in** (~1 KB of a copy that
  averages ~87 KB); decoded by the browser (a GIF that starts dark reads on to its clip, ≤ 32 KB, and shows its first lit frame where the browser has WebCodecs' `ImageDecoder`); kept in IndexedDB per card
  (`odmd-thumbs-<card>`), keyed by path with the file's size, capped at
  24 MB (least recently used out), so a GIF is asked for once, across
  visits. A Details preview, which reads the whole file anyway, re-makes a
  thumbnail whose size no longer matches; the app's own Add, Edit details
  and Delete forget it.

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
  and `/sets/*.txt`. A tap opens one in the editor (`js/pledit.js`, loaded
  the first time one opens), a screen of its own (`#sets/edit`: on a phone
  it replaces the list, with **‹ Playlists** to go back; on a laptop it is
  the right-hand column, empty until something is open). Its header names
  the playlist; the card's own are saved as a new one (**Save as a new playlist**,
  prefilled "Halloween 2"). **+ New playlist** opens an empty one with a
  Name field. The entries show their thumbnails; drag ⠿ (or Alt+↑/↓) to
  reorder, ✕ to remove, a tap picks them for **Arrange** (Shuffle, Sort
  A–Z, Picked to top, Remove picked, Clear). **+ Add GIFs** opens the same
  folder tree as the Library, with a checkbox on every GIF and folder (on a
  laptop beside the entries, the playlist list stepping aside; on a phone
  in their place, **Done** to go back): tick GIFs or whole folders and
  **Add N ticked**, or search (title words, platform chips) and **Add all N
  shown**; a GIF already in the playlist has a ✓ and is not added twice.
  A Formula added from Formulas (**Add to playlist…**: the line is its
  player path, `@formula?psb=…[&a=&b=&c=]&name=…`, the compiled Formula, ≤ 2,048 B) shows by its
  name with a *Formula* tag, no thumbnail. **Add by filter…** is a sheet over the editor that runs the core's
  `Filter` (platforms, categories, title words, **Leave out** flags,
  shuffle with a seed, at most N) over any playlist; its last row is **Add
  N GIFs** / **Replace with N GIFs**. **Save** / **Save & play** (`PUT
  /api/sets/<name>`, then `PUT /api/config {"set": …}` — `""` for All files,
  a root manifest's file name, or a set's name — which the device applies
  in a few seconds), kept in reach at the bottom; a list row's **…** has
  **Play** and **Delete…**. Deleting the active playlist switches the
  player back to All files first. Switching is disabled while status says
  an index is `building`. Entries are GIF paths or Formulas.
- **Settings** — four tabs (`#settings/<tab>`; bare `#settings` is the
  first), only the open one in the page, everything saved as it changes
  with a fading **Saved ✓** in the card's heading:
  - **Playback**: brightness with **Maximum brightness** under it
    (`js/power.js`; default 144, below the level tested safe on the
    supplied power supply; raising it shows a warning, because the panel
    can reset above it), the playlist, **Platforms** (one row — "All
    (60)" or the ones picked — and **Choose…**, a sheet with a search box,
    the playlist's platforms with their counts, those under 5 behind "+N
    more", **Only these** and **All platforms**), shuffle, and **Each GIF
    stays** min – max seconds (the dwell). These apply live (the playlist
    not while the player is still reading one).
  - **Look**: colour and phosphor glow, *Mirror the panel on Now* (this
    browser's choice), the transition with a Preview and the start-up show
    (`js/boot.js`: its length, `boot_show`, and what plays, `boot_formula`:
    the wordmark or a Formula light enough for the show; a Formula's
    preview is pkg-psi's frame with the main module's information line over
    it, `Preview.showBoot`, the panel's `@boot?formula=…`).
  - **Schedule**: time zone, schedule rules, night mode, Follow RePlayOS.
  - **Device** (`js/device.js`, loaded when the tab first opens): **Status**
    (the playlist and its count, the card, uptime, free memory and its
    floor, late frames, refresh rate, GIFs started) with **Reload playlist**
    (`POST /api/reindex`) and **Reboot**; the **Ready to play** card ("9,826
    of 10,133 files prepared", from `GET /api/cache`) with **Prepare
    remaining files**, which works out what the playlist in play is missing
    (`GET /api/cache/list`) and encodes and uploads it in the background, one
    file at a time, with progress, an ETA and a stop button (starting again
    carries on); **Network**: WiFi (with a scan), which needs a reboot, and
    the Display name, live (Save network then says the new `<mdns>.local`
    address, from `/api/status`, and links to it when the page was opened by
    a `.local` name; a name another display already answers to is said
    under the field when it is left and stops Save network — `GET
    /api/name-check` — and the device's 409 for one that slips by is
    explained, nothing saved); **Panel timing** (`js/panel.js`: the HUB75 clock edge,
    latch blanking, driver chip, data clock and minimum refresh, each change
    a trial with Keep/Undo and a countdown, the boot-time ones through
    "Restart now", and Reset to defaults; **Test panel timing**,
    `js/paneltest.js`, loaded on the first click: `POST /api/panel/test` A/B
    questions for the clock edge, then the latch blanking, the pick saved
    with `panel_confirm`; see [`firmware.md`](firmware.md#panel-timing));
    **Firmware** and **Security**; **About**: the address, the firmware
    version and flash variant, and the web app's own version (`www_version`,
    from the pack's header). Firmware updates go over WiFi
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
**Save as new**. **Save as GIF…** in Formulas opens a Formula
here too: one loop of it, drawn as an animation by the same code the panel
runs, titled with its name, to save as new.

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
   and the preview plays it smoothed; if there is nothing to smooth it says
   why — nothing slides, it already moves one pixel at a time, or its frames
   are too quick (a speed of 2× can be) — and looks again after your next
   change. Save keeps the choice.
6. **Save** — title, platform, category, author and "it is a logo", as
   Add GIFs asks for them: the file name is built from them (the number is
   the next free one) and the Studio shows what the Library will list it
   as, or why it can't be named that way. On a device it goes into the
   chosen category, prepared, and the Studio offers to add it to the
   playlist in play; in local mode it downloads.

The pixel work and the GIF writer are Rust (`opendmd-art-web`), in a second
wasm module loaded only when the Studio opens. The game name lists are
published on the project site (`https://opendmd.dev/art-index/v1/<PLATFORM>.json`,
one fetched per platform the first time it is searched, then kept for the
session; the site's demo fetches its own copy), built by
`uv run tools/art-index.py` from the repositories' file lists into
`opendmd-web/art-index/` (committed; not in the www pack); re-run it and
publish the site to pick up new games. The pictures themselves come from
GitHub when you pick a game, so the whole search needs the internet; without
it, Find says so and offers Try again. `v1` is the lists' format: a new
format gets a new path, so an older web app never reads one it doesn't
know. The investigation behind all this
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
to smooth (about one GIF in ten on the stock card). A slow scroll that
moves one pixel a frame is already as smooth as the panel's pixels allow —
there is no whole-pixel position between two frames a pixel apart — so the
Library says so instead of offering the switch; Create Art has the same
switch for an animation you make or edit there, and a GIF that is on stays
on when you edit it there and replace it. The choice is kept on
the card in `/sets/.smooth` and survives clearing the playback cache.
`opendmd-pack smooth analyse <card> [--write]` lists (and can switch on)
every GIF worth smoothing, and `opendmd-pack convert` honours the list.

## Favourites and schedules

**Favourites and schedules.** Right-click a GIF in the Library or a
playlist (its "…" button; on a phone a long press in a playlist) to make it a
favourite, play it less, or — a second step, **Play longer or
loop…** — play it twice as long, for N seconds, or N times — per playlist, so a GIF can
be a favourite in one and play less often in another. Settings → Schedule → Time sets the zone
(the player keeps time over NTP), and its Schedule picks a playlist
and/or brightness for parts of the week ("Weekdays 18:00–23:00 → Consoles")
and a night mode that turns the panel off or dims it; Now shows when either
is in force and can wake the panel for an hour. The saved playlist and
brightness come back when a rule ends.

## Colour and glow

**Colour and glow.** The player can show everything in one
colour like a classic dot-matrix display — Settings → Look → *colour*:
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
transitions and the cache builder would otherwise have.

Follow RePlayOS can show a playlist (a Formula such as the bundled Big clock, `@formula:big-clock`, can be one) in the menus instead of its attract playlist (Settings → Schedule → Follow RePlayOS → *attract playlist*).

There is no Live folder: the clock and the scrolling text (`@clock`, `@text`) are gone, a Formula draws a clock. A playlist line for either is skipped as the playlist is read (an info log line); `POST /api/play` of one is a `415`. The playlist editor drops such lines when it opens a playlist.

The built-in effects (tunnel, rotozoom, starfield, wireframe) are gone from
the firmware and the core: they are Formulas now, in the site's gallery
(`opendmd-web/formulas/*.psi`, one file each; the web app bundles ten of
them, the ones marked `-- bundle`, as `www/formulas.json`, written by
`make web`), where they can be read and changed. A Formula plays, sends to the panel and goes in a playlist like any
GIF (**Show on panel**, **Add to playlist…**). Show on panel and its live
edits send the compiled Formula as bytes (`POST /api/play/formula`, played
as `@formula:~sent`, held in the player's memory: no length rides on a path
and the card is never written, so a Formula of any length the editor takes
shows); a playlist names it by reference, or with no card as a line
`@formula?psb=…` (see [the HTTP API](api.md)). A retired `@fx:` line in a
playlist is skipped with a log line. A playlist of one
Formula, chosen as the attract playlist, shows a Formula in RePlayOS's menus
(`attract` is only `""`; the Settings caption says a playlist can hold Formulas). **Save as GIF…** is in Formulas: see [Formulas](guide/formulas.md).

## Size budget

**Size budget**, checked by `make web` (`tools/web-size.py`) on the gzipped
sizes, which are what crosses the WiFi: the wasm module plus its generated JS
glue at most 116 KB, the hand-written JS and CSS at most 60 KB; the Studio's
own wasm at most 150 KB and its JS and CSS at most 24 KB, loaded only when it
opens; the Add GIFs / Edit details dialogs at most 8 KB, loaded when first
used; the Library's folder tree with the thumbnails (`js/tree.js`,
`js/thumbs.js`, `tree.css`) at most 13 KB and the playlist editor
(`js/pledit.js`) at most 6 KB, loaded with the Library and when a playlist
first opens (12.1 KB of 13 and 5.9 KB of 6 on 2026-10-26, which took the JS and CSS
line from 59.6 to 53.7 KB). Today that is 115.8 KB, 51.3 KB, 110.0 KB, 9.3 KB and 6.3 KB. The game name
lists (2.4 MB, 527 KB gzipped, one platform fetched at a time) are printed
without a budget. `make web` writes a `<file>.gz` beside each file, and the
player sends those in preference.

## The demo

[opendmd.dev/demo](https://opendmd.dev/demo/) is this app with no
player behind it. When nothing answers `GET /api/status` but a
`demo/index.json` sits beside the page, the app loads that card instead of
offering the folder picker: its files are fetched as they are read, writes
stay in the page, and Now starts playing All files by itself. **Use your own card…** in the header swaps in a card folder.

The demo has a player of its own in the page (`js/demo.js`, loaded only
there): it answers the device's settings API (config, schedule, status) for
the browser session, so **Settings** is the device's, every tab and control.
What the settings change shows on the demo's panel on Now: brightness (the
schedule's and night mode's too), colour, glow, the transition, the Formula
look drawn over every frame, how long each GIF stays, the playlist,
platforms and shuffle, and the schedule banner with Wake and Resume. Panel
timing is tried and kept as on a player, with nothing to see. Firmware
updates, WiFi and restarting keep their controls and say in a line that the
demo can't do them. A new browser session starts from a new player's
settings.

The Formula editor on the site (the demo's and `/f/`) has two aids the
player's copy leaves out (`js/fcost.js`, imported only there, and its wasm
module `pkg-psi-lang/`, opendmd-psi-web's `lang` build; neither is in the
www pack). **What costs most**: a bar in the gutter beside each line for
its share of the panel's work, and the compiler's notes on the cost, each
with a **Line N** button; shown by itself once a Formula is past light, and
for any other on its button (a light one says so). **Tidy up** lays the
text out as `psi fmt` does, as one edit the browser's undo takes back. The
module is the whole compiler (~121 KB gzipped), so it loads only for a
heavy Formula or one of the two buttons.

The demo card is OpenDMD's own Formulas, recorded by the simulator, the
CC0 Kenney Pixel Platformer sprites (a playlist of sprites, "Sprites")
(`tools/demo-card.py`). `make site` builds the whole public site into
`target/site`: the README as the landing page, these docs as HTML, and the
app with the demo card under `demo/` (`tools/site.py`). `make site-serve`
serves it on :8081; `make site-deploy` puts it on
[opendmd.dev](https://opendmd.dev/) (the `opendmd-site` Worker on
Cloudflare, `site/worker/`; see [deploying.md](deploying.md)).

**A formula's page** (`/f/#gallery=<slug>`, from `opendmd-web/share/f/`)
is the app's own Formulas editor on the demo's files (`js/formulas.js`
`editorPage`: the live preview, errors, sliders, cheat sheet and settings),
with Open on my panel, Copy code and Submit to the gallery
(`js/fsubmit.js`, the app's form too). A link names a Formula, never
carries its text: `#gallery=<slug>` (the gallery's tiles), or `#open` (the
Psi reference's Open in editor, which leaves the example in the site's own
storage). **Open on my panel** opens the player's app in its editor:
`http://<address>/#gallery=<slug>` for a gallery Formula as it is (the app
has the bundled ones, else fetches `opendmd.dev/formulas/src/<slug>.psi`,
CORS `*`), or, once changed, a window on `http://<address>/#remix` that
says it is ready to the page that opened it and gets `{name, src, a, b, c}`
back by `postMessage` (accepted only from `https://opendmd.dev`, or the
app's own origin for the demo). The app saves it under Your formulas as it
arrives (`js/fstore.js`: on the panel's card, else in the browser; a
bundled one needs nothing, the same text is not saved twice, one that
doesn't compile is kept as an unsaved draft), then opens it in the
editor. There are no anonymous short links:
sharing is submitting to the gallery.

**Earlier versions and drafts** (`js/fhist.js`, imported when needed): a
ring of the last 10 texts per Formula in the browser's `localStorage`
(`odmd.psi.v.<key>`; the key is a saved Formula's id, `g<slug>` for one from
the gallery, `n…` for a new one, moved to the id on its first save), kept on
each save and as a Formula arrives (and, on `/f/`, as it goes to a panel),
each at most 32 KB; the oldest go first when storage is full. A version
goes back through the editor's minimal-range edit as typed, so undo has
it. `/f/` keeps its draft per Formula (`odmd.psi.fd.<key>`, with the text
it started from) and offers it back on return; `#open`, read once, starts
again from that text. The editor's preview can be held (`hold` on the
view, ms): the loop draws that moment until it plays on.

**The formula gallery** (`/formulas/`) is the app's examples plus the
files merged into the public gallery repository's `gallery/`
(github.com/opendmd/opendmd.github.io) by pull request. A workflow there
(`site/public-repo/.github/`, installed by `make gallery-repo-publish`,
which otherwise leaves `gallery/` alone) checks each
contribution with `opendmd-gallery` (`opendmd-gallery/`: the path, the
text, the header, the licence, that it compiles and its cost on the panel;
its threat model is in `site/public-repo/.github/gallery-check/README.md`).
`make site GALLERY=<a checkout's gallery/>` checks every file again before
publishing it and draws the thumbnails; the release workflow fetches it.
`make site-smoke` drives both pages in Chromium.

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
everything but the app to a real device. `GET /mock/io` (mock only, no
firmware route) counts the card-file requests in flight and at most and the
bytes actually sent (a reader that hangs up stops the count); `?reset=1`
zeroes it, `&kbps=N` paces card files in the firmware's 2 KB pieces, so the
smoke measures the thumbnails' early cut-off as a device would see it;
`&fail503=N`, `&fail500=N`, `&drop=N` fail the next N card-file GETs. Card
and app files go out as the device's stream task sends them: one at a
time, two waiting, a full queue for 2 s a `503` with `Retry-After: 1`.
`--proxy` passes a sized answer on as it comes, so a reader that hangs up
closes the device's connection too. `tools/library-load.py --device URL`
drives the Library, the thumbnails and the editor through it against a
real player and reports requests, bytes, connections, 503s, `/api/status`
latency, the heap floor, misses and `streams_aborted` per scenario.
