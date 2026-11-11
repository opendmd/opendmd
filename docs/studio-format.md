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

`body`: `w` 128, `h` 32, `plays` (how many times it plays; absent: forever; its GIF says N plays as a loop count of N - 1, which browsers keep: the panel's player loops a GIF for as long as its playlist keeps it on, whatever its loop count), `frames` (each `{ms}`; the Studio writes tens of ms, as a GIF keeps hundredths of a second),
`layers` (each `{name, moves, cels}`: a layer that moves has a cel a frame,
one held still has one), `palette` and, since S1, `colours` and `keys`.

- **Layers and frames.** `layers[0]` is the bottom. A pixel shows the
  topmost shown layer's key that is not 0; 0 in every layer is off. A layer
  with `hide: true` is not shown, and is in no GIF, link or send: what is
  seen is what is sent. A frame's `ms` is a whole number from 20 to 60,000;
  a layer's `name` is 60 characters at most; `moves` is `true` or `false`.

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
- Caps: 256 frames, 16 layers, 4,096 colours; 4 MiB of JSON for a piece,
  1 MiB for any other record.

**An optional field at its default is absent.** A writer never writes
`hide: false`, `keys: 1`, `plays: 0` or an empty `note`, and lists a
piece's or a palette's colours by rising key: the same content would
otherwise hash two ways. `check()` refuses `hide` that is not `true`,
`keys: 1`, an empty `note` and colours out of key order. It still takes
`plays: 0`, which every piece written before S2 carries; the Studio reads
it as no `plays` when the piece opens, so such a piece's hash changes once,
at its next save, and never again.

A pixel whose key the piece has no colour for is off, as it is shown: the
Studio reads it as 0 when the cel is first read.

**Reading pixels is bounded.** A pixel field is never inflated past what it
can hold: a whole panel, two bytes a pixel, at most (a piece's cel: its own
size exactly). Text too long to deflate from that much is refused unread,
and a stream that gives more is dropped at one byte past it, so a small
file cannot hold a picture that inflates to gigabytes.

A palette's body is `{colours, source}`, with the same `colours` shape.

The other bodies:

- **stamp**: `w` (1 to 128), `h` (1 to 32), `frames` (1 to 64, each `{px}`
  with `ms` on every frame or none), and its `colours` or the `palette` they
  come from; `keys: 2` as for a piece. Placed, a stamp's pixels go flat into
  the cel drawn on, and the piece's `from` names the stamp once.
- **font**: `cell {w, h}`, `line`, `first`, `count`, `adv` (an advance a
  glyph) and `atlas {cols, w, h, px}`, one key a pixel, 1 inked.
- **template**: a piece's structure (`w`, `h`, `frames`, `layers`), its
  `palette` or `colours`, and `guides` (`{kind: safe | text | art, name, x,
  y, w, h}`, text ones with `font`, `align`, `sample`). A piece made from
  one takes its layers and frames and its palette's colours as its own, and
  its `from` names the template.
- **pack**: `members`, each `{id, rev}`, each once, never the pack itself.

## Files

A file is `{format: "opendmd-studio/1", root, records}`: `.dmdart` holds a
record (and its palette if that is the user's own), `.dmdpack` a pack and
every member at the rev it lists; a backup is a pack tagged `backup` of
everything in the Library, whose own record is not kept when it is opened.
Saving a file raises the record's rev, as a download does, and writes that
revision, with who made it and its licence as asked then.

Opening one trusts nothing in it: 16 MB and 4,160 records at most, every
record through `check()` and its hash worked out again, each id once, the
root in the file, a pack's members all in it at their revs and no pack
holding itself however far round. Then it is shown, and nothing is stored
until the user says Add. By id: the same hash is skipped; a higher rev is
an update (offered); other content comes in as a copy with a new id whose
`from` names the file's record, and what in the file points at it points at
the copy; a starter record is never replaced. A record deleted here comes
back when a file holds it again.

**Its GIF.** Up to 256 colours a GIF frame holds them all, over one table.
Past 256, each frame is written as a base and corrections: first the whole
picture over 256 of its colours, each other colour drawn as its nearest
there (so a player that starts the GIF, or loops it, never shows a pixel
unpainted), then only the pixels not yet exact, 255 more colours at a
time (the most used first), the rest transparent, 20 ms each; the last holds the rest of the
frame's time. A frame shorter than 20 ms for each of those GIF frames lasts
20 ms each instead. Nothing is lost, and the panel plays it as it is, with no
quick-start copy and no Smooth motion; "Download fitted to 256 colours"
writes only the base, which the panel can prepare and smooth. The 256 are
picked for spread, as Create Art picks a picture's colours: the most used
first, then again and again whichever colour is furthest from all those
picked so far, weighted by log(1 + its uses), so a rare colour far from the
rest is kept and a crowd of near-twins is not. The GIF writers are
`opendmd-studio-web`.

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
  (`Conflict`); a put naming no version over a record deleted here brings it
  back. The Studio then goes on from the stored record if only its
  rev or name moved (taking the other tab's name unless it was renamed here
  too), and keeps its own work as a copy if the pixels moved.
- **The undo journal** is the browser's alone: never in a record, a file or
  a sync, so its shape may change freely; a journal of another shape is
  dropped when the piece opens (the piece itself is untouched), and so is
  one that does not lead to a valid piece. A step written at or below the
  step the stored record stands at clears the record's `hash` in `meta`
  until the next save, so a journal whose earlier steps were replaced is
  never replayed over that record. A pixel
  step names its cel by layer and frame key and carries the indices and the
  keys before and after; a step that changes the piece's colours carries
  what it added and took out (`c: {add, del}`); a snapshot restored is one
  step holding both bodies; a change of structure (a frame or layer added,
  removed or moved, a frame's time, the plays, a layer renamed, hidden,
  held still or made to move) is one step holding the structure before and
  after and only the cels one side has, so the cels both keep are not
  copied. Steps are numbered back to back per piece (500
  at most, or about 8 MB); `meta`
  holds `cur` (the last step done) and `at` with `hash` (the step the stored
  record stands at, and its hash). Opening a piece replays (or unwinds) the
  steps from `at` to `cur`; if the record was written without them (another
  tab), its journal is dropped. One tab at a time keeps a piece's journal
  (`navigator.locks`); a second tab on the same piece still draws and saves.
- The starter palettes come from `studio-starter/starter.dmdpack`, served
  beside the page. They are read-only, recognised by the pack's member ids;
  Den 16 is built in, with a fixed id.
