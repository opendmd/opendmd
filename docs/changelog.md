# Changelog

What's new in each OpenDMD release, newest first. These are the highlights:
what you'd notice on the panel and in the web app. To get the newest
version, [update over WiFi](../../docs/guide/updating.md) or use the
[installer](../../docs/guide/install.md). Betas are early versions of the
next release, and are marked.

<!--
How this file works (docs/deploying.md, "The changelog"):

  ## <version> · <YYYY-MM-DD>     a release: one summary line, then 2 to 5 bullets
  ## <any other heading>          a group of short entries (the betas before a release)
  ### <version> · <YYYY-MM-DD>    a short entry in a group: a sentence or two

Newest first. The site adds the date's wording, the Beta and Latest labels and
each release's link to GitHub from the heading. An entry for a version newer
than the newest published release is held back until that release is out, so
it can be written ahead. Write for a panel's owner: GIFs, Formulas, playlists,
the web app. No internals.
-->

## 1.1.0-beta.3 · 2026-11-13

Sprites from your own pictures.

- **Picture…** in the Formula editor makes a sprite from a picture you
  choose, drop or paste: crop it, size it up to 64 × 32 and pick 2, 4 or
  16 colours, with what each costs the Formula said as you go.
- A sprite's picture editor can replace it from a picture, keeping the key
  characters you gave its colours. The picture never leaves your browser.

## 1.1.0-beta.2 · 2026-11-11

Formulas keep better time on the panel.

- A looping Formula now repeats at exactly its loop's length on the panel.
- Two new words in Psi: `smooth` asks for a Formula to be drawn on every
  refresh of the panel, and `phase` says where a frame is in the loop.
- A `smooth` Formula too heavy to draw on every refresh plays at every
  second one, where it used to be refused.
- Fixes for playlist edits that could be lost when they were made while the
  playlist was being saved or refreshed.
- A new OpenDMD mark and two-colour wordmark, in the web app and on this
  site.

## 1.1.0-beta.1 · 2026-11-10

Tools for a big library: tags, stars and smart playlists.

- Tags and stars of your own on any GIF, a search that understands them,
  and smart playlists that are built from a search and keep up with it.
- Move, rename, tag, rate or delete many GIFs at once. Playlists can be
  renamed and duplicated, and the Library adds straight to the playlist you
  choose.
- Schedule rules can have dates, with Halloween and Christmas in one press,
  and night mode can run from sunset to sunrise.
- The display can play from its card with WiFi turned off, and the
  start-up show can be one of your own Formulas.
- A display on 1.0.0 or 1.0.1 needs one install over USB from the
  [installer](../../docs/guide/install.md) to move to this version. It
  keeps its WiFi details and settings.

## 1.0.1 · 2026-11-08

A better Formula editor, and a guide to writing Formulas.

- The Formula editor shows a swatch and a picker for every colour, lets you
  paint a sprite's picture and edit a palette in place, suggests names as
  you type and offers a fix for common mistakes.
- On this site the editor can also show what costs a Formula the most time,
  and tidy up its layout.
- New guide pages: Make a Formula, over three short pages, How Psi thinks,
  and The editor.
- A look made from one of your Formulas follows it: it changes when you
  save the Formula and is cleared when you delete it.
- Plainer wording across the web app, the guide and the display's own
  messages.

## 1.0.0 · 2026-11-07

The first stable release, and the first with its source public on GitHub.

- Everything from the betas below: your GIFs played without tearing, a web
  app for the Library, playlists and schedules, Formulas with their gallery
  and editor, Create Art, and updates over WiFi.
- The panel shows an Installing picture while an update installs, where it
  used to go dark.

## Road to 1.0

The betas that led up to 1.0.0, a line or two each.

### 1.0.0-beta.17 · 2026-11-06

A **Get beta updates** switch in Settings. The Now view follows the panel
as it changes, without waiting to ask. The panel says Updating while a
firmware update comes in.

### 1.0.0-beta.16 · 2026-11-06

More room for the firmware, which took one install over USB, and a
smaller web app. The display stays responsive when the network or its
memory is busy, and work or school WiFi is supported again.

### 1.0.0-beta.15 · 2026-11-05

Only GIFs and Formulas play now: the built-in clock and scrolling text are
retired, and a Formula draws the clock. Formulas can read the daylight, the
sun, the moon and the season where the panel is. Create Art on this site
takes your own GIFs and videos. A round of security hardening before 1.0.

### 1.0.0-beta.14 · 2026-11-04

Room for bigger Formulas: more steps, longer text, and scenes that switch
on and off. After an update the web app always runs its new version. A fix
for ad blockers that stopped the Formulas view from loading.

### 1.0.0-beta.13 · 2026-11-04

The Formula engine rebuilt, with the picture kept apart from card and
network work so it stays steady while files move. Create Art on this site,
with **Send to my panel**. **Add to my panel** from the gallery, and **Keep
on the panel** for a Formula you want to watch. New Formulas, among them
Neon city and Wormhole run. A fix for displays with very large libraries
restarting.

### 1.0.0-beta.12 · 2026-10-29

The built-in effects became Formulas you can open and change. Formulas got
much faster on the display, can draw lines and dots, and can run at 100
frames a second. A Formula has a settings sheet of its own, can go in a
playlist, and can be kept as a file on the card. New Formulas, among them
Endless road, Tunnel, Starfield and Wireframe cube.

### 1.0.0-beta.11 · 2026-10-26

No more tearing: the picture is drawn in step with the panel's refresh.
The Library became a folder tree, and playlists are edited by browsing,
with thumbnails. Any Formula can be the panel's look, drawn over every GIF.

### 1.0.0-beta.10 · 2026-10-26

GIFs open quickly however many are in a folder. Formulas can draw digits
and letters.

### 1.0.0-beta.9 · 2026-10-26

The card runs at a safe speed by default, with a faster one to opt into. A
Big clock Formula in the gallery.

### 1.0.0-beta.8 · 2026-10-25

One Formula can be used inside another, and there is a reference for
writing Formulas with an AI model.

### 1.0.0-beta.7 · 2026-10-25

Formulas can be drawn over the GIF that is playing, as a look, with layers
and blends. More free memory on the display.

### 1.0.0-beta.6 · 2026-10-24

Formulas play on the panel itself, with **Show on panel** and live edits.
GIFs with more than 256 colours play exactly as they are.

### 1.0.0-beta.5 · 2026-10-22

A Formula merged into the gallery appears on the site without waiting for a
release.

### 1.0.0-beta.4 · 2026-10-22

A page for each Formula on this site, and a public gallery that takes
contributions.

### 1.0.0-beta.3 · 2026-10-22

Formulas arrive: a view of their own in the web app, with a gallery, an
editor and **Save as GIF**.

### 1.0.0-beta.2 · 2026-10-22

A refreshed web app: Settings in tabs, a playlist editor on a screen of its
own, and Now and the Library reworked for phones. A display can be renamed
without a restart.

### 1.0.0-beta.1 · 2026-10-22

Library search across every GIF, favourites from the Now view, Recently
played with Previous and Play again, and a guided test for the panel's
timing.

## 0.3.0 · 2026-09-24

Where it started: the first version that could update itself over WiFi.

- Plays the display's GIFs from its microSD card, with transitions between
  them.
- A web app served by the display to manage it, with a live preview of the
  panel.
- Playlists with schedules and a night mode.
- Follow RePlayOS: the panel shows the game being played on a RePlayOS box.
