# The card

A DMDoS card root looks like:

```
gifs/<Category>/<file>.gif    18 categories: Consoles, Arcade, Pinball_Short,
                              Halloween, XMAS, Other, "NEW 2021", ...
logos/
filelist.txt
filelist.hall                 seasonal subsets, same format
filelist.xmas
```

`filelist.txt` is one path per line: LF-separated, leading slash, no BOM, in
insertion order rather than sorted. The stock firmware reads only
`/filelist.txt` and plays it sequentially. OpenDMD is tolerant on input (BOM,
CRLF, blank lines, trailing whitespace) and canonical on output (one path per
line, LF-terminated — byte-identical to the shipped card file).

Every GIF in the archive is 128x32 except two strays (132x32 and 135x32).
Mismatched sizes are centred and cropped, never resampled — it is pixel art and
any scaling filter ruins it.

## Prepared copies

A prepared copy is the same animation re-encoded as a standard GIF that is
cheap for the player: every frame already composited into one global
palette, each frame after the first sent as only the rectangle that changed,
and the LZW codes capped at 9 bits (an `OPENDMD1LZW` application extension
says so). The player decodes it with a small streaming decoder in **~16 KB**
of heap including the 12 KB canvas, against ~27 KB for the original through
the same decoder and ~54 KB through the `gif` crate — headroom the ESP32
spends on WiFi and serving — and it reads ~40% fewer bytes off the card:
across the stock card the copies are 0.61x the size of the GIFs (960 MB for
1,575 MB). Any viewer, the browser included, plays them. All but 38 of
11,039 files decode to exactly the frames the GIF player shows; those 38
have more than 255 colours and are quantised (play the original for any that
look wrong). The format is [`docs/cache-format.md`](cache-format.md);
the experiment that chose it, [`docs/gifslim-experiment.md`](gifslim-experiment.md).

**Users see GIFs and playlists; prepared copies are a cache.** Playlists
only ever name GIFs. For every `/gifs/<Category>/<name>.gif` it opens, the
firmware first tries the copy at `/cache/<Category>/<xx>/<name>.gif` (`<xx>`
= low byte of FNV-1a-32 of the file name, 256 buckets per category, so a
lookup scans ~15 entries, never the 3,781-file `gifs/Consoles`) and plays it
if it opens and is a GIF; otherwise the original. A miss costs ~5-6 ms (one
failed open in a small directory, measured on the rig). Originals play
through the same small decoder at its full 12-bit table; the few it does
not handle (interlaced frames, `Previous` disposal) fall back to the `gif`
crate. To prepare a card copy:

```bash
# cache/<Category>/<xx>/<name>.gif for every GIF; skips files already
# prepared; --verify plays copy and original and compares every frame
make pack ARGS="convert /path/to/card --verify"

# onto a running player instead: the files (or use the web app's
# Settings -> Playback cache, which prepares them in the browser)
uv run tools/card-sync.py <host> --from /path/to/card --paths list.txt
```

**The player also prepares them itself.** While the panel is dark for the
night, paused, or has no playlist, it walks the playlist in play and writes
the missing copies with a streaming encoder that needs ~43 KB of heap —
byte-identical to `opendmd-pack` and the browser for every file of 256
colours or fewer (11,001 of the stock card's 11,047; the 38 with more
colours, 5 that only the `gif` crate decodes and 8 GIFs that will not decode
are left for the browser, which has the quantiser and the memory).
`auto_prepare` in `PUT /api/config` is `"idle"` (the default: only then),
`"always"` (also slowly while a prepared file plays) or `"off"`; Settings →
Playback cache shows it as one sentence ("Preparing files automatically
when the panel is idle · 4,210 of 10,133 · about 3 nights left") with the
option "also while playing (slower)". On the rig the stock card goes at
~1.3-1.4 s per file at full speed (0.7-0.75 files a second: 620 in 15
min, then 217 in 5 min), so a card with none prepared needs about 4 hours
— one night.

That is all: no manifest to write, nothing to select. `GET /api/cache`
reports how many of the active playlist's entries have a copy. The copies
leave `gifs/` and `filelist.txt` alone, so the card still works with stock
DMDoS. **Cards prepared by earlier builds** have an `odm/` tree of the retired
pre-rendered `.odm` copies: the player ignores it (and says so once at
boot); remove it with `DELETE /api/files/odm?recursive=1`, repeated until
the reply says `"done":true` (see [`docs/deploying.md`](deploying.md)).

## Tag grammar

Filenames encode tags as `PLATFORM_Title_Words_NN[_FLAG].gif`. It is a
convention rather than a format, so parsing is best-effort and derived from the
real ~10k-file card. Tokens are split on `_`:

| Filename | Platform | Title | Variant | Flags |
|---|---|---|---|---|
| `SNES_Donkey_Kong_Country_05.gif` | SNES | Donkey Kong Country | 5 | |
| `MD_Sonic3_RattenJager.gif` | MD | Sonic3 | | RattenJager |
| `SNES_Asterix01.gif` | SNES | Asterix | 1 | |
| `NINTENDO_DS_Mario_Kart_DS_01..gif` | NDS | Mario Kart DS | 1 | |
| `NES_Unknow_01_Invert.gif` | NES | Unknow | 1 | Invert |
| `HALLOWEEN_ARCADE_GoldenAxe16.gif` | ARCADE | GoldenAxe | 16 | HALLOWEEN |
| `MD_LOGO_Sega31.gif` | MD | Sega | 31 | LOGO |
| `Nes_Gradius.gif` | — | Nes Gradius | | |

Rules, in order:

1. A leading `HALLOWEEN` / `XMAS` becomes a flag only when a known platform
   prefix follows it; otherwise it *is* the platform. The directory category is
   the authoritative source of seasonality either way.
2. Multi-token prefixes (`NINTENDO_DS` -> `NDS`, `SEGA_MEGACD`, `VIRTUAL_BOY`,
   `SEGA_PICO`), then the known single-token list, then any unknown token that
   *looks* like a platform: `A-Z`, `0-9`, `-`, at least one letter — so `PICO-8`
   works and `Nes` doesn't. Anything else leaves the platform unset and the
   whole stem becomes the title.
3. Keyword flags (`INVERT`, `INTRO`, `LOGO`, `SLIDE`, `STORY`, ...) directly
   after the platform.
4. Trailing tags right-to-left: keyword flags, a standalone all-digit token as
   the variant, and author handles.
5. Failing that, trailing digits glued to the last word are the variant — but
   only two or more of them, because a single glued digit is almost always a
   sequel number (`Sonic3` stays whole; `Asterix01` becomes Asterix variant 1).

Author handles can't be told from title words in general (`StarWars01`,
`SuperMario` and `RattenJager` are all trailing CamelCase), so a token counts as
an author only if it starts with a lowercase letter (`sephirot68`, `clivefrog`)
or is in a small known list, and only when dropping it leaves a non-empty
title. Flags are stored verbatim; every flag comparison is case-insensitive.
`normalize_title` (lowercase, alphanumerics only) is the hook for fuzzy
matching a user's own collection list.

Against the real card this parses 10,162 entries with a non-empty title each,
and 3,784 of 3,786 `gifs/Consoles` entries get a platform (99.95%; the two
misses are `Nes_Gradius.gif` and `NEs_Captain_America...gif`).

**Writing a name** (the web app's Add GIFs and Edit details;
`manifest::NameFields` / `build_name` in the core, behind its `names`
feature) is the inverse, and is only allowed when the name reads back as
exactly the fields it was built from:

- The form is `[SEASON_]PLATFORM_Title_Words_NN[_author][_FLAG…].gif` under
  `/gifs/<Category>/`. The number is always written (two digits, last before
  the author), so a title may end in a number: `MD_Streets_of_Rage_2_03`.
- Title words are split on spaces; each may use printable ASCII except
  `_` (the separator) and FAT's `" * / : < > ? \ |`, and the title may not
  start with `.`. No platform means no number and no author (the parser
  reads the whole name as the title then), and the first word may not look
  like a platform (`THE END` would read as platform `THE`).
- An author is one token of `A-Za-z0-9.-`, three or more long, not a keyword
  and not all digits; it is written starting lower-case (`Ross` → `ross`)
  unless it is a known handle (`RattenJager`), because that is how the parser
  recognises one.
- A title word that the parser would take for a tag is refused rather than
  written: `Toy Story` (→ `Toy` + `Story`), `Intro Stage`. A category is 48
  bytes at most, a file name 120.

| Fields | Name |
|---|---|
| Super Metroid · SNES · Consoles · 1 | `/gifs/Consoles/SNES_Super_Metroid_01.gif` |
| Streets of Rage 2 · MD · Consoles · 3 · ross | `/gifs/Consoles/MD_Streets_of_Rage_2_03_ross.gif` |
| Sunset · no platform · Mine | `/gifs/Mine/Sunset.gif` |
| Toy Story · SNES | refused (reads back as "Toy") |

Every name on the stock card read into fields and built back reads as the
same fields (`make test-card`): 10,145 of 10,162; the other 17 cannot be
rebuilt as they stand (16 have a title word that is also a keyword, like the
`Story` of `PINBALL_STORY_IndianaJones_Story02`, one an author with a space
in it).
