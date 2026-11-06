-- The gallery store (opendmd-notes docs/research/site-cloudflare.md "S6 as built"): the
-- formula gallery's entries, so /formulas/, /formulas/src/<stem>.psi and
-- /api/gallery are served from here (src/gallery.ts).
--
-- OpenDMD's examples (source 'bundle' for the `-- bundle` ones the panel
-- comes with, 'example' for the rest) are seeded from the deployed site's
-- formulas/gallery.json, checked by opendmd-gallery at the build; the
-- contributions (source 'gallery') are synced from the public repo's
-- gallery/ by the Worker's cron and checked by the Worker. A row is always
-- a version that passed: a newer one that fails is noted in
-- gallery_refused and the row kept (the last good one).
create table gallery (
  source   text not null,                -- bundle | example | gallery
  slug     text not null,                -- the file name without .psi
  file     text not null,                -- the whole file: the header and the formula
  src      text not null,                -- the formula (the file after its header)
  name     text not null,                -- its `name` line
  about    text not null,                -- its `about` line
  author   text not null,                -- the header's `-- author:`
  licence  text not null,                -- CC0-1.0 | CC-BY-4.0
  look     text,                         -- JSON [blend, amount] for a `look` line, else null
  controls text not null,                -- JSON: its controls' labels, slot a, b, c in order
  esp32_us integer not null,             -- estimated µs a frame on the panel (with its still cache)
  loop_ms  integer not null,
  inputs   integer not null,
  sha      text not null,                -- git's blob id of `file` (sha1, hex)
  thumb    blob,                         -- 128 × 32 PNG (contributions; the examples' are the site's formulas/examples/)
  status   text not null default 'ok',   -- ok | gone (no longer in gallery/)
  updated  integer not null,             -- unix seconds
  primary key (source, slug)
);

-- The latest version of a gallery/ file that failed the checks (its row in
-- gallery, if any, is the last version that passed). Cleared when the file
-- passes or goes.
create table gallery_refused (
  slug     text primary key,
  sha      text not null,
  problems text not null,                -- JSON array of sentences, as opendmd-gallery writes them
  seen     integer not null
);

-- Small state: the commit last synced, when, the examples' build, and the
-- sync and seed claims (unix seconds).
create table meta (
  key   text primary key,
  value text not null
);
