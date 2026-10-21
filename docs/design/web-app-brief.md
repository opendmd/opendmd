# The web app: visual brief (2026-10-02)

Status: the brief for a visual pass (the web app "feels a
bit clunky; the panel feels snappy"). Written from the smoke
screenshots at 1280 px and 390 px (`target/web-smoke/*.png`, device and
mock). Implementation lands in phases, each reviewed against this page.
Budgets stay as they are (`tools/web-size.py`); this is mostly CSS and
wording, and a few structural changes per view.

## What makes it feel clunky

Looking at every view side by side, the problems repeat:

1. **Everything is a box in a box.** Cards inside cards, each with a 1 px
   border on a near-black background, all the same weight. Nothing is
   primary, so the eye has nowhere to land. The Health block on Now is ten
   bordered tiles inside a bordered card inside the page.
2. **Too many buttons, all shouting equally.** A Playlists row carries
   Open · Activate · Delete; a playlist entry carries … ↑ ↓ ×; Settings has
   three orange Save buttons on one screen. The orange primary appears five
   or six times per view (Skip, Make art, Send to panel, Save playback,
   Save transition, Save start-up), so it stops meaning "the thing to do".
3. **Plumbing shows.** Raw card paths under the title on Now and in Details
   (`/gifs/Other/OTHER_Consoles18.gif`), a "/gifs/… (card path)" input on
   Now, folder names as category labels (`BEST_OF_TOP_30`, `Pinball_Short`,
   `_bench`), flag codes as placeholders (`INVERT,LOGO`), "0.3.0 4mb" in the
   header, "sta · opendmd.local". The product model says users see GIFs and
   playlists; the names on screen should be human.
4. **Inconsistent voice.** `brightness` / `colour` / `phosphor glow` in
   lowercase next to "Brightness" in sentence case; "preview · in step with
   the panel" and a checkbox "preview follows the panel" saying the same
   thing twice; helper paragraphs three lines long above a single select.
5. **Phone is a stretched laptop.** Tiles become 400 px tall, the Library
   header is five stacked rows before the first GIF, Health is a full
   screen of numbers nobody checks on a phone.
6. **The preview is not the hero.** The Now page is the panel, but the
   preview gets the same frame as the Health tiles. Make art's preview and
   Now's preview render the dot matrix at different scales.

## Principles

- **One primary action per view**, orange, and it is the thing the user
  came for: Now = Pause/Skip (one pair), Library = Send to panel (in
  Details), Playlists = Save, Settings = none (settings apply live),
  Make art = Save to the card. Everything else is quiet (text or outline),
  destructive actions live in menus and dialogs, red only there.
- **Two surface levels, not three.** Page, and card. No card inside a card.
  Group with spacing and a heading, not with borders. Borders are for
  inputs and the rare separator; tiles lose theirs.
- **A spacing and type scale**, and nothing off it: 4/8/12/16/24/32 px;
  13 px captions, 15 px body, 17 px titles, 24 px page title; one weight
  change for emphasis, never colour plus weight plus size.
- **Human names everywhere a user reads.** Categories: `BEST_OF_TOP_30` →
  "Best of Top 30", `Pinball_Short` → "Pinball Short", a leading `_` is a
  hidden/system list and is shown last in muted type. Flags: INVERT →
  "Inverted", LOGO → "Logo", etc. (the map exists in `manifest.rs`'s
  KEYWORD_FLAGS; mirror it in JS). Paths appear in exactly one place: a
  small "File" line in Details and in Make art's Save, in mono, muted.
- **Sentence case labels, no colons, no trailing periods on captions.**
  Form labels above fields, 13 px muted. Helper text one line; anything
  longer becomes a "?" tooltip or is cut.
- **Settings apply as you change them** (they already do on the device) and
  show a quiet "Saved" tick in the card heading for two seconds. No Save
  buttons. Preview buttons stay.
- **The dot matrix renders at one scale rule everywhere:** the widest
  whole-pixel multiple that fits, with the same dot gap, so Now, Library
  Details and Make art look like the same panel.
- **Motion only where it carries meaning:** a 150 ms fade for a panel that
  appears, the "live" dot's pulse, nothing else. No hover growth.
- **Phone first for layout decisions:** each view must read well at 390 px
  with at most two rows of controls above the content.

## Per view

### Now
- The preview fills the card width, no inner border, caption below in one
  line: "Zelda · Other · 18 of 10,136 · Following RePlayOS" with the follow
  state as a small chip, and "Live" as the green dot only.
- Controls under it in one row: Pause/Skip as a pair (primary), Brightness
  slider with the value, nothing else. The "Send to panel" path input goes;
  sending is done from the Library.
- Health collapses to one line of three figures (uptime, free memory,
  misses) with a "More" disclosure for the rest, as a plain two-column
  list, not tiles. On a phone the disclosure is closed by default.

### Library
- Header in two rows: row 1 search (with the source switch inside its
  right end as a small segmented control), count; row 2 the category tabs
  (human names) as a horizontal scroller with the platform chips on the
  same row after a divider, or as a second scroller only when a category
  has platforms. Add GIFs and Make art become one "+ Add" split: "Add
  GIFs…" and "Make art" in a small menu, so the header has one button.
- Rows: a 32 px preview of the first frame on the left is wanted but costs
  card reads; land it behind a "Thumbnails" toggle in Settings, default
  off on the device and on in local mode, lazy per visible row with a
  budget of two in flight. The platform tag moves right-aligned, muted.
  The "…" button appears on hover/focus and always on touch.
- Details: title, then one line "SNES · Consoles · by Author", preview,
  then the actions in two rows: Send to panel (primary) + Add to playlist;
  Edit in Make art + Smooth motion as a toggle row; a "…" menu for
  Favourite/Hide in, Edit details, Delete. The file line last, small.
- Web sources: the results grid keeps its look; the narrowing row becomes
  part of row 2 (replacing the tabs) so nothing jumps.

### Playlists
- The list: name, count, a "Playing" chip on the active one; the whole row
  is a button that opens it; Activate and Delete move to a "…" menu.
- The editor: entries with a drag handle (keyboard: alt+arrow), the
  platform tag muted, a single "…" for favourite/hide/duration/remove.
  Save (primary) + "Save & activate" quiet; Shuffle/Sort/Picked to top in a
  single "Arrange" menu.
- Build from filters: human category names, "Flags to drop" as chips with
  human names, platforms as a chip picker not a text field.

### Settings
- Four cards, flat: Playback, Transition, Start-up, Power; Advanced as a
  disclosure at the end holding cache/schedule/network/health. Fields in a
  two-column grid on laptop, one on phone; sentence case; autosave.
- The brightness slider shows the ceiling (Power) as a marker.

### Make art
- The six numbered steps stay (they help), but as a single column on
  phone and two balanced columns on laptop (Import+Frame left, Look+Text
  right, Preview+Save full width below). Step headings become
  "1 Import" in the title style, not small caps.
- The preview uses the shared matrix scale; the timeline keeps its
  thumbnails; the controls row under it groups Trim (Start/End/Delete/Undo)
  and Motion (Speed, Reverse, Ping-pong, Loop, Smooth motion) with a gap.
- The game-art finder shows results as the tile grid from the Library's
  find module, not a list and then tiles.

### Header and nav
- Header: "OpenDMD" wordmark, then the device name only; address and
  version move to Settings → About. On a device, a tiny status dot: green
  live, amber reconnecting.
- Side nav on laptop, bottom bar on phone, as now; the active item gets
  the orange bar only, no background fill.

## Phases

1. **Foundation** (one commit): tokens (scale, two surfaces, no nested
   borders), button system (primary/quiet/destructive, sizes), label voice,
   human names for categories and flags, path lines, header, matrix scale
   rule. No view restructuring yet; every view should already look calmer.
2. **Now and Settings** (autosave, health disclosure, hero preview).
3. **Library and Playlists** (header rows, Details actions, list rows and
   menus, builder chips).
4. **Make art** (layout, grouped controls, finder tiles).
5. **Phone pass and motion**, then a final screenshot review.

Each phase: `make web-smoke` green, screenshots at both widths for review,
sizes reported. Smoke selectors change where the DOM changes; keep ids.

## Not in this pass

- Thumbnails in the Library rows on the device beyond the toggle above.
- Any new feature. Feeds, OTA and the hosted platform have their own docs.
