# Studio starter pack

The content a new OpenDMD Studio library starts with: palettes, stamps, two
pixel fonts and four 128x32 templates, on a retro gaming theme. Everything
here was drawn for OpenDMD and is dedicated to the public domain (see
"Licence" below).

| Kind | Count | What |
|---|---|---|
| Palettes | 16 | 12 classic hardware palettes, 4 of our own tuned for LED panels |
| Stamps | 35 | pickups, hearts, aliens, a ship, effects, icons, digits, lettering; 10 animated |
| Fonts | 2 | Tiny 3x5 and Arcade 5x7, ASCII 32 to 126, monospaced |
| Templates | 4 | Scoreboard, Arcade marquee (animated), CRT bezel, Now playing |

## Files

- `src/<kind>s/<slug>.json`: one record per file, the source of truth. Edit
  these by hand.
- `build.py`: checks every record and writes the pack.
- `starter.dmdpack`: the built pack, committed so the Studio can ship it
  without running Python.

```bash
make studio-starter                          # check + write starter.dmdpack
make studio-starter ARGS=--check             # fail if starter.dmdpack is stale
make studio-starter ARGS="--sheets out/"     # also draw starter-sheet.png and starter-sample.png
```

(`uv run studio-starter/build.py` is the same thing.) The contact sheet shows
every palette, every frame of every stamp, both fonts, the templates with and
without their guides' sample text, and a 128x32 scene made of the pack's own
stamps and fonts on LED dots. Look at it after any change to the art: at 1x
on a panel a stamp needs a strong outline and few colours.

## Source records

Each source file is a record in the Studio's envelope, without the three
fields the build works out (`id`, `rev`, `hash`) and without dates:

```json
{
  "type": "stamp",
  "name": "Heart, full",
  "tags": ["hud", "life"],
  "note": "A full life.",
  "author": {"name": "OpenDMD", "account": null},
  "licence": "CC0-1.0",
  "from": [],
  "body": {
    "w": 7, "h": 6, "palette": "panel-16",
    "frames": [
      {"rows": [
        ".55.55.",
        "5255555",
        "5555555",
        ".55555.",
        "..555..",
        "...5..."
      ]}
    ]
  }
}
```

- The file name is the record's **slug**. Records point at each other by slug
  in the source (`"palette": "panel-16"`, a guide's `"font": "arcade-5x7"`);
  the build turns slugs into ids.
- **Pixels are rows of characters**, one per pixel: `.` is see-through (key
  0), `1` to `9` are palette keys 1 to 9, `a` to `z` keys 10 to 35. Stamps
  and templates here all use the `panel-16` palette (keys 1 to 15: `1` black,
  `2` white, `3` silver, `4` steel, `5` red, `6` orange, `7` yellow, `8`
  green, `9` pine, `a` cyan, `b` blue, `c` navy, `d` magenta, `e` violet,
  `f` rust).
- **Palettes**: `body.colours` is a list of `{"k": key, "hex": "#rrggbb",
  "name": ...}`. Keys start at 1; 0 always means see-through.
- **Stamps**: `w`, `h`, `palette`, and `frames`, each with `rows` and
  optionally `ms`. Frames with times are an animation; frames without are a
  set to pick from (the digit stamps: frame *n* is the digit *n*).
- **Fonts**: `cell` (`w`, `h`), `advance` (pixels from one character to the
  next), `line` (pixels from one line to the next) and `glyphs`, a map from
  each character to its rows (`1` is ink).
- **Templates**: a piece body (`w`, `h`, `palette`, `plays`, `frames`,
  `layers` with one cel per frame for a layer that `moves`, else one) plus
  `guides`: rectangles of kind `safe` (keep the picture inside), `art` (put a
  picture or stamps here) or `text` (with `font`, `align` and a `sample`).
- `note` is one line saying what the record is or where it comes from.
- `from` is empty for our own work. A hardware palette carries one entry,
  `{"external": false, "note": "..."}`, naming the source of its values:
  `external` is false because a colour table is a fact about a machine, not
  someone else's artwork.

The build refuses a record with a wrong size, a pixel whose key is not in its
palette, a font missing any of ASCII 32 to 126 (or with two identical
glyphs), a guide off the panel or too small for its sample, an unknown
reference, or any author or licence but the pack's own.

## The pack file

`starter.dmdpack` is a bundle in the Studio's one file format,
`opendmd-studio/1`: `{"format", "root", "records"}`, with `root` the id of
the pack record (the last record), whose `body.members` lists every other
record as `{id, rev}`. Records are written one per line, palettes and fonts
before the stamps and templates that use them.

What the build adds to each source record:

- `id`: the kind's prefix (`pl`, `st`, `fn`, `tp`, `pk`), `_`, and the first
  32 hex digits of SHA-256 of `opendmd-studio-starter:<kind>/<slug>`. Fixed
  for as long as the slug is, so a Studio can recognise starter records.
- `rev`: 1.
- `hash`: `sha256-` plus the SHA-256 of the body as canonical JSON (keys
  sorted, no spaces, UTF-8, non-ASCII characters not escaped).
- `created`, `modified`: one fixed date, so the build repeats byte for byte.

and what it changes in the body:

- every `rows` becomes `px`: the pixels as one byte per pixel (the palette
  key, 0 see-through), row by row, deflated (zlib format) and base64-encoded.
  A stamp frame is `{"px", "ms"?}`; a template layer's `cels` is a list of
  `px` strings.
- a font's `glyphs` become an `atlas`: one picture of all 95 characters, 16
  to a row in ASCII order, each in its own `cell`; `atlas` holds `cols`,
  `w`, `h` and `px` (0 or 1 per pixel). The body also has `first` (32),
  `count` (95), `line`, and `adv`, one advance per character.
- slugs become ids.

## Where the hardware palettes come from

Colour tables are facts about machines. Where a machine has exact digital
values the palette uses them; where it does not (anything that made analogue
video or had a passive LCD), the palette says so in its `from` note.

| Palette | Source |
|---|---|
| NES (RGB PPU) | The 2C03 RGB picture chip's documented 3-bit levels per channel (NESdev wiki, "PPU palettes"), each scaled `level * 255 / 7`; the 50 distinct colours. The home console's 2C02 makes composite video and has no exact RGB values. |
| Game Boy DMG | The usual emulator convention for the four shades (`#0f380f` to `#9bbc0f`): an approximation of a reflective LCD. |
| Game Boy Pocket | Four evenly spaced greys: an approximation. |
| C64 (Pepto) | Philip "Pepto" Timmermann's 2001 calculation from measured VIC-II levels (pepto.de/projects/colorvic/2001). |
| C64 (Colodore) | Pepto's later Colodore model at its default settings. |
| ZX Spectrum | One bit per channel plus a brightness bit; normal intensity drawn at `0xD7`, a common convention. |
| CGA / EGA 16, CGA Mode 4 palettes | IBM's RGBI scheme (`0xAA` per channel, `+0x55` with intensity, colour 6 brown). |
| EGA 64 | Two bits per channel, in the card's colour-number order. |
| MSX / TMS9918 | Converted from the chip's datasheet levels for a CRT, as tabulated on Wikipedia's TMS9918 page. |
| Atari 2600 (NTSC) | Our own model of the TIA's 16 hues by 8 brightness levels (the formula is in `from`): not a measurement. |

The four palettes of our own (Panel 16, Neon Arcade, Phosphor Amber,
Phosphor Green) avoid dim, close-together colours, which a HUB75 panel
crushes at low brightness.

## Originality

The stamps, fonts and templates are original: nothing is traced from or
drawn to resemble a game's sprites, characters or logos, and no product's
palette is included. Keep it that way when adding to the pack; when in
doubt, make it different.

## Licence

The starter pack (everything in this directory's `src/` and
`starter.dmdpack`) is dedicated to the public domain under
[CC0 1.0 Universal](https://creativecommons.org/publicdomain/zero/1.0/): to
the extent possible under law, the OpenDMD authors have waived all copyright
and related rights to it. Use it for anything, without credit. Every record
says so in its `licence` field (`CC0-1.0`). `build.py` is code and is under
the repository's licence (MIT OR Apache-2.0).
