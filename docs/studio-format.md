# The Studio's records

How the Studio (`opendmd.dev/studio/`, `opendmd-web/share/studio/`) keeps
what people make: the record envelope, a piece's body, the hash rule, and
what stays in the browser. `store.js` is the code; this page is the rule any
other reader or writer (`studio-starter/build.py`, a later server) follows.

## Records

Everything is a record with one envelope: `type` (`piece`, `palette`,
`stamp`, `font`, `shade`, `template`, `pack`), `id` (the type's two-letter
prefix, `_`, 32 hex digits; random, never reused, never made from a name),
`rev` (an integer that rises only at a snapshot or a download, never on an
autosave), `hash`, `name`, `tags`, an optional one-line `note`, `created`,
`modified` (ISO 8601, to the second, UTC), `author` (`{name, account}`,
`account` null until there are accounts), `licence` (null is "not chosen",
which is not a licence; else `CC0-1.0` or `CC-BY-4.0`) and `from`.

`from` lists what a record was made from. An entry that names a record
carries `id`, `rev` and that revision's `hash`, so it says which content it
was, plus `name`, `author` and `licence`; anything else (an imported file)
is `{external: true, note}`. A copy made because another tab changed the
same draft gets a new `id` and a new `created`, and its `from` names the
draft.

Records point at each other by `id`, never by name or position.

## A piece

`body`: `w` 128, `h` 32, `plays` (0 forever), `frames` (each `{ms}`),
`layers` (each `{name, moves, cels}`: a layer that moves has a cel a frame,
one held still has one), `palette` and, since S1, `colours` and `keys`.

- **Its colours are its own.** `colours` lists `{k, hex}`: keys from 1, each
  once, `#rrggbb` in lower case, no colour twice. Key 0 is off, the LED dark.
  A new piece has none; a colour picked joins the piece in the step that
  first draws with it. A piece written before S1 has no `colours`, and its
  colours are its palette's.
- `palette` is the swatch set it was drawn from (Den 16, a starter palette,
  one of the user's), by id. A palette is a set to draw from, never a limit.
- **Pixels** are colour keys, row by row, zlib-deflated, in base64: one byte
  a pixel. A piece that needs a key past 255 says `keys: 2`, and then every
  cel is two bytes a pixel, **high byte first**. Without `keys` (or with
  `keys: 1`) a piece is one byte a pixel, as every piece before S1 is, so
  those stay valid unchanged.
- Layers and frames carry `key`, eight hex digits, made when the layer or
  frame is, so what points at one (later) survives reordering. The key is
  part of the body, so it is in the hash. Pieces written before S1 have none.
- Caps: 256 frames, 16 layers, 4,096 colours, 1 MiB of JSON per record.

A palette's body is `{colours, source}`, with the same `colours` shape.

**Its GIF.** Up to 256 colours a GIF frame holds them all, over one table.
Past 256, each frame is written as a base and corrections: first the whole
picture over its 256 most used colours, each other colour drawn as its
nearest there (so a player that starts the GIF, or loops it, never shows a
pixel unpainted), then only the pixels not yet exact, 255 more colours at a
time, the rest transparent, 20 ms each; the last holds the rest of the
frame's time. A frame shorter than 20 ms for each of those GIF frames lasts
20 ms each instead. Nothing is lost, and the panel plays it as it is, with no
quick-start copy and no Smooth motion; "Download fitted to 256 colours"
writes only the base, which the panel can prepare and smooth.

## The hash

`hash` is `sha256-` and the SHA-256, in lower-case hex, of the body's
canonical text:

1. Replace every pixel field (each string in a `cels` list, each `px`
   string, anywhere in the body) with its bytes uncompressed, as lower-case
   hex. For a two-byte piece the bytes are as stored, high byte first: the
   same rule, so every hash made before two-byte pieces existed is
   unchanged, and `keys: 2` is in the body the hash covers.
2. Write the result as canonical JSON: keys sorted, no spaces, UTF-8,
   non-ASCII characters not escaped. In Python: `json.dumps(v,
   sort_keys=True, separators=(",", ":"), ensure_ascii=False)`.

The hash is of the pixels, not of how one browser happened to compress them.

**No floats, ever.** Canonical JSON holds only JSON's own values, and every
number is an integer (`Number.isSafeInteger`); `canon()` refuses anything
else, `undefined` included. Different languages print fractions
differently, so a fraction would make the same record hash differently in
two places. A record that needs one says it in whole units: milliseconds,
not seconds; a percentage as 0 to 100.

## In the browser

`LocalStore` keeps records in IndexedDB (`opendmd-studio`): `records`,
`revs` (frozen revisions; a snapshot's has a `label`), `refs` (what points at
what), `journal` and `meta`.

- **Writes are versioned.** Every head carries `ver`, an opaque store
  version that any write moves on, a rename in another tab included. A put
  of a record that exists must name the `ver` it last saw, or it is refused
  (`Conflict`). The Studio then goes on from the stored record if only its
  rev or name moved (taking the other tab's name unless it was renamed here
  too), and keeps its own work as a copy if the pixels moved.
- **The undo journal** is the browser's alone: never in a record, a file or
  a sync. Steps are numbered back to back per piece (500 at most); `meta`
  holds `cur` (the last step done) and `at` with `hash` (the step the stored
  record stands at, and its hash). Opening a piece replays (or unwinds) the
  steps from `at` to `cur`; if the record was written without them (another
  tab), its journal is dropped. One tab at a time keeps a piece's journal
  (`navigator.locks`); a second tab on the same piece still draws and saves.
- The starter palettes come from `studio-starter/starter.dmdpack`, served
  beside the page. They are read-only, recognised by the pack's member ids;
  Den 16 is built in, with a fixed id.
