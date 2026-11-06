-- Short links (opendmd-notes docs/research/site-cloudflare.md §5, "S5 as built"): one row
-- per shared Formula, never edited (a changed Formula is a new row).
-- Applied with `cf d1 migrations apply` (remote: make site-d1-migrate; local
-- dev and tests apply it themselves).
create table formulas (
  id        text primary key,            -- the short id: 10 Crockford base32 characters of hash (longer on a collision)
  hash      text not null unique,        -- sha256 of src, hex
  src       text not null,               -- the source as shared (CRLF -> LF, trailing blank space cut), <= 4096 B
  name      text,                        -- its `name` line (null when it has none)
  about     text,                        -- its `about` line
  controls  text,                        -- pkg-psi controls(): {name, about, look, controls: […]} as JSON
  esp32_us  integer not null,            -- estimated µs a frame on the panel
  bytes     integer not null,
  status    text not null default 'ok',  -- ok | hidden (taken down: 404)
  created   integer not null,            -- unix seconds
  source    text not null default 'share' -- share (POST /api/f); gallery | bundle later (S6)
);
create index formulas_created on formulas(created desc) where status = 'ok';
