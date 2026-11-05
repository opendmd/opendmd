-- Anonymous usage counts (src/counts.ts, the public page /stats): how many
-- installs the web installer made and how many update checks the web app
-- made, a day. Daily totals only: one row per (day, kind, version,
-- variant), its count incremented in place. Nothing about a request is
-- kept: no address, user agent, cookie, referrer or device id, and no row
-- per request. Every value is checked by the Worker before it is written
-- (a version-like string or 'other', a known variant or 'other'), and a
-- day takes at most 40 versions of a kind (the rest go to 'other'), so the
-- table stays small whatever anyone sends.
create table counts (
  day      text not null,               -- YYYY-MM-DD, UTC
  kind     text not null,               -- install | check
  version  text not null,               -- 1.0.0-beta.14 and the like; unknown (not sent) | other (not a version)
  variant  text not null,               -- the flash layout: 4mb | 8mb | unknown | other
  n        integer not null default 0,
  primary key (day, kind, version, variant)
) without rowid;
