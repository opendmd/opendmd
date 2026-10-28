# /// script
# requires-python = ">=3.11"
# dependencies = ["pynacl>=1.5"]
# ///
"""A fake OpenDMD ESP32 device, for testing the browser management UI without
hardware.

    uv run tools/mock-device.py --root ~/scratch/dmdos-card [--overlay DIR] \
        [--www opendmd-web/www] [--host 127.0.0.1] [--port 8080] [--dwell SECS] \
        [--strict] [--quiet] [--ota-pub firmware/keys/opendmd.pub]

    uv run tools/mock-device.py --proxy http://192.168.1.69 [--www ...] [--port 8080]

Two modes.

**Card mode** (`--root`) fakes the whole device: the JSON API in
`firmware/src/http.rs`, the card file server in `firmware/src/files.rs`, and a
simulated now-playing loop, all served on top of a local copy of an SD card
(`--root`, an extracted card: `gifs/`, `cache/` (prepared copies),
`filelist.txt`, `filelist.hall`/`.xmas`, `sets/`). `--www` (default `<repo>/opendmd-web/www`) is
what a browser hitting `/` gets.

**Proxy mode** (`--proxy URL`) does the opposite: it serves `--www` locally
(same static-file rules as card mode) and forwards every other request,
verbatim, to a real device at `URL`. This is how a locally built UI gets
tested same-origin against real hardware without uploading it to the device's
card first.

# The overlay (`--overlay DIR`, card mode only)

A copy-on-write layer sitting in front of `--root`. Every write (an uploaded
file, an installed manifest, a `.idx` sidecar, a delete) goes into the
overlay; `--root` is never touched. Every read checks the overlay first, then
falls back to `--root`. Deleting a file that only exists in `--root` cannot
remove anything there, so it is recorded as an in-memory tombstone instead:
the file is hidden from reads and directory listings for the rest of this
run, exactly as if it were gone, but `--root` is untouched and the tombstone
evaporates the next time the tool starts. Directory listings merge both
layers by name, overlay entries winning, tombstoned names dropped. Without
`--overlay`, writes land directly in `--root` (this is also what real
firmware does: a set you install lands under `<SD_MOUNT>/sets/`).

# Firmware behaviours deliberately reproduced here

- **JSON key order** matches `http.rs`'s hand-rolled `format!` strings, field
  for field (Python dicts preserve insertion order, and `json.dumps` respects
  it, so building the response as an ordered dict is enough).
- **Route precedence**, including a quirk of ESP-IDF's httpd that the
  firmware relies on: a URI can match a wildcard registered for a *different*
  HTTP method than the one requested. Concretely, `/api/sets/*` and
  `/api/manifest` are only ever registered for `PUT`; a `GET` to either
  therefore does not 404 from "no such route" but instead falls through to
  the catch-all `/*` (`GET`), which does a `www/` lookup and 404s from there
  if nothing matches. A `DELETE` to `/api/sets/*`, on the other hand, matches
  no `DELETE`-registered pattern *and* no `GET` catch-all applies to it (the
  catch-all is `GET`-only) except that ESP-IDF's "URI matches, wrong method"
  rule fires against the `PUT` registration, giving 405. This mock
  reimplements that exact algorithm (see `dispatch`) rather than
  special-casing `/api/sets/*` and `/api/manifest` by hand.
- **Manifest installs** always delete `<dest>.idx` *before* writing the new
  manifest (`files::remove_index_beside` then `files::replace`) — the
  crash-safety ordering from `files.rs`, kept here for fidelity even though
  the mock cannot crash mid-write.
- **`is_manifest_path`**: only `filelist.*` directly at the card root, or any
  file directly inside `sets/` (not nested), and never `.idx`/`.tmp`/`.part`.
- **A new `set` is checked before anything is saved** (it must open), then
  the settings are saved, then the playlist is rebuilt for a changed
  `set`/`platforms`/`shuffle`. A rebuild that still fails (`set` = "" on a
  card with neither default manifest) is a 500 with the settings kept, as in
  `main.rs`. A platform filter (new, or kept across a `set` change) under
  which the playlist would have no entries is a 400 `no GIFs in the playlist
  match the platform filter "<filter>"; nothing was saved`, nothing saved
  (the firmware builds it, finds it empty and rebuilds the previous one).
- **`hostname`** is trimmed and lowercased, then must be 1-32 of a-z, 0-9
  and `-`, not starting or ending with `-` (`opendmd_core::hostname::valid`),
  else a 400; `""` leaves it as it was. A change is live at once, as on the
  device (no `reboot_required`). Status `mdns` (the name mDNS answers to) is
  always the hostname here: no other display to clash with.
- **A name another display has is refused.** `--taken NAME=IP,...`
  (default `other-display=192.168.1.42`; `""` for none) are the names
  other devices answer to (`<name>.local`). `GET /api/name-check?name=`
  answers `{"name","valid","taken","by"}` as the firmware does (the name
  trimmed and lowercased; `taken` null for an invalid name; the current
  name is never taken), after the device's wait: up to 1.2 s for a free
  name (`--name-query`), at once for a taken one. `PUT /api/config` with a
  changed `hostname` that is taken is a 409 `{"error":"name taken:
  <name>.local already answers from <ip>","name","by"}`, nothing saved.
  The firmware says "could not ask" (`taken` null, the save allowed) in
  setup mode or with its network worker busy; not modelled.
- **`GET /setup`** serves `firmware/web/setup.html` with its two template
  values filled in, as the firmware does (setup mode itself is not modelled).
- **Smooth motion's list, `/sets/.smooth`** (one card-absolute GIF path a
  line; `opendmd_core::tween::LIST_PATH`), needs nothing of its own, on the
  device or here: `GET /sets/.smooth` serves it like any file under
  `/sets/`, and `PUT /api/files/sets/.smooth` is — like anything directly in
  `sets/` — a manifest install (no `.idx` to delete, never the active
  playlist, so just the swap); the web app lists only `sets/*.txt` as
  playlists. Its smoothed copies are ordinary uploads under `cache/`; the
  simulated player drops its cached frame delays whenever a copy or a GIF
  is written, since a smoothed copy has more frames. A GIF written (`PUT
  /api/files/gifs/…`) drops its `cache/.failed` line, as the device does.
- **`POST /api/files/move {"from","to"}`** renames a GIF under `gifs/` as
  `http.rs` `move_file` does: 400 unless both are card-absolute
  `/gifs/….gif` paths (no tab or line break) and differ, 404 if `from` is
  not there, 409 if `to` is (a case-only rename is not a clash); the
  prepared copy moves to the new name's bucket (a stale copy there goes
  first), and the GIF's lines in `/sets/.smooth` and `/cache/.failed` are
  renamed. `{ok, from, to, copy: "moved"|"removed"|"none", smooth, failed}`.
  Playlists and overrides are the client's to rewrite.
- **No `GET`/`DELETE /api/sets/*` route** — sets are read at
  `GET /sets/<name>.txt` and deleted at `DELETE /api/files/sets/<name>.txt`.
- **`.gz` preference is `www/`-only**, and unconditional (no `Accept-Encoding`
  check) — the content type still comes from the logical, un-gzipped name.
- **Directory listings are unsorted** (`os.scandir` order, standing in for
  FAT order) and sizes are opt-in (`?sizes=1`), matching the firmware's
  quadratic-`stat`-avoidance note in `files.rs`.
- `GET /api/config` never echoes `wifi_pass`, only `wifi_pass_set`.
- **`GET /mock/io`** (mock only, no firmware route): card-file requests
  (`/gifs/`, `/cache/`) in flight now and at most, how many, the bytes
  actually sent and how many were cut off by the client; `?reset=1` zeroes
  them, `&kbps=N` sends card files in 2 KB pieces at that rate (as the
  firmware's `IO_CHUNK` streaming), so a reader that hangs up early is
  measured as a device would see it. web-smoke's thumbnail checks use it.
- **`transition` / `transition_ms`** in the config (`"dissolve"`, 400 by
  default): a name `opendmd_core::compositor::Kind::from_name` accepts
  (stored normalised) and 20..=2000 ms, else a 400. The mock plays nothing,
  so it only stores them; the page's previews are what show one.
- **The default manifest** is `filelist.txt`: `set` = `""` plays it,
  `"filelist.*"` selects that root manifest, anything else `sets/<set>.txt`.
  An obsolete `filelist.odm.txt` is ignored and refused as a set. A set whose
  manifest does not exist is a **404 `{"error":"set not found"}` with nothing
  saved**; a `/`, `\\` or `..` in it is a 400. `manifest` in status, the 409 on
  deleting the active manifest and the "is this install active" check all use
  the resolved path.
- **The playback cache.** `GET /api/cache` is `{total, prepared, state,
  count_ms, auto}`: `total` the playlist length, `prepared` how many of its entries
  have a prepared copy at their cache path (`cache/<Category>/<xx>/X.gif`)
  or play as they are (a `cache/.failed` line whose reason starts "more than
  256 colours": no copy on purpose, done), counted in a background thread after any change (an upload or delete under
  `cache/`, a forget, a GIF delete, a playlist rebuild) — the first request
  after one answers `"counting"` with the previous figure (null before the
  first count), as the device does. `GET /api/cache/list` is every copy's
  path; `POST /api/cache/forget {"path":"/gifs/…"}` deletes a GIF's copy;
  `DELETE /api/files/gifs/…` takes the copy with it. `GET /api/cache/failed`
  is `[{path, reason}]` from `cache/.failed`, `POST
  /api/cache/failed {"path","reason"}` appends a line to it (once per path;
  `{ok, path, added}`; one that plays as it is recounts) and `DELETE /api/cache/failed` removes it
  (`{ok, cleared}`): the GIFs that can never be prepared. A legacy `odm/`
  tree (the old prepared format) is ignored: never counted, listed or
  played. `DELETE /api/files/<dir>?recursive=1` removes a directory and
  everything under it, for `cache/` and `odm/` (and anything below them)
  only: `{ok, path, removed, done}` (files removed; `done` always true here —
  the firmware works at most ~4 s a request and says false: ask again);
  anywhere else a 400, a
  missing directory a 404.
- **Auto prepare** (`auto_prepare` in the config: `"always"` by default,
  as on the device since 1.0.0-beta.11, `"idle"`, `"off"`, anything else a
  400). `/api/cache` ends with
  `"auto":{mode,state,reason,policy,session,failed,avg_full_s,avg_drip_s,
  eta_s,current}` and status with `"cache":{state,prepared,total}`. The
  builder is simulated and **writes nothing**: its state follows the mock's
  own situation -- `off` -> idle/"off"; an empty playlist -> idle/"no
  playlist"; nothing counted yet -> idle; nothing missing -> `done`; the
  player paused (manually or by night mode "off") -> running at `full`
  (one file / 2 s); playing with `always` -> running at `drip` (one file /
  6 s); playing with `idle` -> paused/"playing". While running it walks the
  last count's missing list (`current`), every 25th file a known failure
  (`failed`), the rest added to `prepared` and `session`; `avg_*_s` are the
  fixed paces once one file went at that pace, `eta_s` = files left x
  `avg_full_s`. Any recount (anything that invalidates the cache count)
  drops the simulated copies and failures (not `session`), so `prepared` is
  the real count of copies unless the simulation has run since the last
  change -- web-smoke sets `idle` first thing, so its cache checks, taken
  while the mock plays, see real numbers only.
- **`/filelist*`** serves every root manifest (`filelist.txt`, `.hall`,
  `.xmas`, an obsolete `.odm.txt`) as `text/plain`, `no-cache`; a `/` after
  the name is a 404.
- **`POST /api/play {"path"}`** puts the file on "the panel" now (an
  off-playlist `now_playing`, ordinal and `gifs_started` bumped), and the
  schedule resumes at the next dwell. 404 if it does not open, 415 if it does
  not start with `GIF8` (the firmware's error text there comes from the
  decoder; here it is a fixed one), 400 for a relative or escaping path.
- **Recently played** (opendmd_core::history): every file start is
  recorded (distinct paths, newest first, at most 16 / 640 B stored; not
  `@power-on` or `@card:`); `GET /api/status?recent=1` adds `recent` and
  `recent_at` after `paused`, and `POST /api/skip?back=1` (Previous) plays
  the entry one older than the one on screen without recording it (404
  `nothing earlier` at the oldest).
- **Maximum brightness.** `max_brightness` (16..255, default 128 =
  `BRIGHTNESS_SAFE_MAX`) is a config field; a `brightness` above it (or the
  same request's) is a 400, lowering it lowers `brightness` in the same save,
  and the PUT reply and `/api/config` carry both. Status has `max_brightness`,
  `brightness_safe_max` and `safe_start_until` (30 for the mock's first 30 s,
  then null; the mock's "panel" is not held down, there is none).
- **Looks and live items.** `look` (`colour`, `amber`, `red`, `green`, `blue`,
  `white`; `color` is stored as `colour`), `glow` (`off`, `short`, `long`) and
  `attract` (`""`, `"clock"`, `"text:<message>"`, at most 205 characters, no
  control characters, or `"fx:<effect>[?options]"`) are config fields, validated as the firmware does (a
  400, nothing saved) and applied live, no reindex. Status ends with `look`,
  `glow`, `glow_held` (12288 while glow is not `off` and the simulated player
  is playing, else 0) and `attract_live` (null, or `@clock` / `@text?msg=..`
  / `@fx:..` while following RePlayOS in its attract state with `attract`
  set).
  `POST /api/play` also takes live items: `@clock[?style=..&seconds=1&date=1&
  h12=1&color=RRGGBB]` and `@text?msg=..[&speed=ms&font=..&color=..&scroll=0]`
  and the effects `@fx:tunnel|rotozoom|starfield|wire[?..]` and the start-up
  show `@boot[?len=short|full|card&v=&ip=&ap=&note=]` (10 ms frames over
  1300 / 5600 ms, or the status card's 16384 ms loop; titled "Startup") (plus
  `@power-on`, `@card:`); any other `@...` is a 415. They read no file:
  a clock is 1000 ms frames, a text is frames of `speed` ms looping over
  (text width + 129) frames (one 1000 ms frame for a `scroll=0` text that
  fits; an effect's loop is 480/960/240/480 frames of 20 ms at speed 1,
  divided by `speed` (1..4, default 2): 10 ms frames and twice as many by
  default (`fps=100`), 40 ms and half as many at `fps=25`). A playlist line may be one too; `/api/cache` never counts them as
  needing a copy.
  **Formulas** (`@fx:psi?src=..[&a=&b=&c=&fps=&name=]`, a non-empty `src`;
  `name`, a playlist entry's, is its title): 20 ms
  frames (10 at `fps=100`, 40 at 25) over the source's `loop N` seconds
  (default 8); not compiled here (any `src` plays). A formula over the
  formula on screen replaces it in place (no new ordinal, a fresh dwell);
  `"live":true` does only that and is a 409 when no formula is on screen
  (400 with any other path); status names the new formula, as the
  firmware does. Over 2,048 B a formula's path is a 413.
  `"over":true` draws the formula over the file on screen instead (the
  firmware's Formula look from the editor): `{"ok":true,"path":..,"over":true}`,
  until the ordinal changes; `""` takes it off; `live` with none on is a
  409. Config `look_formula` (`""` or a formula's path; not compiled here)
  and `look_formula_name` (at most 60 characters), `look_blend` (over,
  screen, add, multiply, max, min) and `look_amount` (0..=100); status `formula_look`
  (null, or `{"source":"setting"|"over_gif","held":20480,...}`).
- **`GET /api/stat?path=`** is `{path, exists, size, is_dir}`, `size` null for
  a directory or a miss, `/` a directory.
- **`/api/status`** has `index_state` (`ready`, or `unavailable` when the
  manifest does not open; `building` is never observable, as on the device,
  because a rebuild holds the lock a status read needs), `www_version` and
  `www` (see the web app, below).
- **The web app.** On the device it is a www pack in flash
  (`docs/www-pack.md`); the mock serves `--www` as files, the way it always
  did (`<file>.gz` preferred, unconditionally), and does not read the pack
  format (the format has one implementation, in `opendmd-core`). `GET
  /api/www` is `{version, bytes, entries, ok, error}`: `version` the first
  line of `--www`/version.txt (what `make web` also writes into the pack's
  header), `bytes` the size of `www.pack` beside `--www` (0 if none),
  `entries` the files the pack would hold. `/api/status` carries the same
  object as `www`, and `www_version`. `PUT /api/www` wants a
  `Content-Length` (411), at most 1,310,720 B (413) and the `ODWP` magic
  (400), a header whose total length is not the body's (400); it replies
  as `GET` does and changes nothing served, but from then on reports the
  pushed header's version (bytes 20.., length at 18) as `version` and
  `www_version`, as the device would, so an update through the web app's
  "Check for updates" can be followed. `staged` is
  null until a PUT, then whether the last one would have been staged on the
  card (true unless `--sd` is `unreadable` or `none`; null again after one
  refused at its header, as on the device).
- **The card** (`--sd`): `ok` (the default); `unreadable`, a card that
  mounts but whose filesystem cannot be read (exFAT; firmware `sd::probe`):
  `sd` is `{mounted: true, readable: false, error: "card not readable: …"}`
  and nothing plays (`now_playing.path` "", `entries` 0, `index_state`
  `unavailable`); `none`, no card (`mounted: false`). Only `/api/status`,
  `PUT /api/www`'s `staged` and `GET /api/firmware`'s `pending`/`last`/`log`
  follow it; the card copy is still served. `POST /api/card/format`
  (`{"confirm":true}`) formats an unreadable card: 202, `sd.formatting`
  true for 3 s, then the card reads (`--sd ok` from then on); a readable
  card, no card or a format already running is a 409, as on the device.
- **Card speed** (`card_speed`: `safe` (the default), `fast`, `full` (API
  only), else a 400 `card_speed must be safe, fast or full`; the PUT reply
  adds `card_speed_applied` `now`, or `at_mount` with `--sd none`). Status
  `sd` has `clock_khz` (null with no card), `fallback`, `probe`,
  `slowdowns`, `recoveries`: safe is 20000 and no fallback, full 40000,
  fast 40000, or 20000 + fallback while the card is recorded slow.
  `--sd slowed` starts a readable card set to `fast` that has already been
  slowed (fallback, 20000, `slowdowns` 1, `probe` "slowed");
  `POST /api/card/full-speed` forgets the record (409 when `card_speed` is
  safe) and answers `{ok, applied, clock_khz}`; `POST /api/reboot?sd_io_fail=3`
  records the card slow too, `?sd_fallback=clear` forgets it.
- **Restarts**: `/api/status` `restarts` starts at `power_on`; a reboot,
  a firmware update, a panel trial undone and the `sd_io_fail` chain add
  theirs (newest first, at most 8), and `reset_reason`/`reset_note` follow
  the newest.
- **Security** (docs/api.md "Security"), checked in `route` before any
  handler: a `POST`/`PUT`/`DELETE` whose `Origin` names another host[:port]
  than `Host` (scheme dropped, `:80` dropped, case ignored) is a 403
  `{"error":"cross-origin request refused"}` (no `Origin`: allowed); with
  `api_key` set (`PUT /api/config`, 8..64 of `A-Za-z0-9._~-`, `""` clears;
  `GET /api/config` shows only `api_key_set`) every `/api/*` request and
  every `GET` of `/gifs/*`, `/cache/*`, `/sets/*`, `/filelist*` needs
  `Authorization: Bearer <key>` or `?key=<key>`, else 401 `{"error":"API key
  required"}` + `WWW-Authenticate: Bearer` (the web app's own files never
  do; setup mode is not modelled); and the JSON routes (`JSON_ROUTES`) with
  a non-empty body whose `Content-Type` is not `application/json` are a 415
  `{"error":"Content-Type must be application/json"}`.
- **Firmware update** (docs/ota.md): `GET /api/firmware` is `{version,
  build, variant, flash_size, pending, last, log}` (`log`: the card's
  `/update/log.txt`, its last ≤1 KB of whole lines, or null; `variant` is
  `--variant`, 4mb by default, `flash_size` its flash, or `--flash-size`); `PUT /api/firmware` takes a signed bundle
  and verifies it as the firmware does against `--ota-pub`
  (default `firmware/keys/opendmd.pub`): 411 without a length, 413 over
  140 B + the layout's factory partition (2,359,296 B, 3,145,728 B for 8mb),
  422 `{"error":"<reason>"}` for any failure, a bundle for the other flash
  layout and an image whose app descriptor is not `"<version> <variant>"`
  among them (nothing changes). Success: `{"ok":true,"version",
  "rebooting":true}`, then a simulated restart: for `RESTART_SECS` every
  `/api/*` request is a 503 `{"error":"restarting"}` (`Retry-After: 1`),
  then the player "boots" again reporting the new `version` in
  `/api/status` and `/api/firmware`, with `last` an updater-style line,
  `log` the lines the firmware and the updater would have appended, and
  `pending` null. Nothing is written to disk.
- **Panel timing** (`opendmd_core::panel_timing`, docs/firmware.md): the
  config's `panel` (`clk_phase`, `latch_blanking` 1..4, `driver`,
  `i2s_speed` 8/16/20, `min_refresh` 30..240), `panel_now`, `panel_next`
  (what a restart would run, else null), `panel_trial_s`. A `panel` object is a trial (the clock
  edge and blanking at once; a driver, clock or refresh change staged for
  the next `POST /api/reboot`, which runs it on trial), reverting after
  `--panel-trial` seconds (60) unless `panel_confirm: true`; `false` undoes
  it (a boot trial by a simulated restart: the API 503s for
  `RESTART_SECS`); `panel: null` saves the defaults. Bad values are 400s
  with the firmware's messages, a confirm with no trial a 409. The mock has
  no panel, so nothing visible changes. `POST /api/panel/test`
  (`{"setting"}` / `{"stop":true}`) keeps the test's state and replies
  (A and B, `left_s`), a 409 during a trial, with a change staged for a
  restart, or `clk_phase` on a rising-edge-only chip; a stop, any config
  PUT or 120 s end it.
- **JSON bodies over 8 KB** (`PUT /api/config`, `POST /api/play`) are a 413
  `{"error":"body is N B, the limit is 8192 B"}`, judged on `Content-Length`.
- **Per-GIF overrides** (`opendmd_core::overrides`): a playlist's `.meta`
  sidecar (`/filelist.meta`, `/filelist.<x>.meta`, `/sets/<n>.meta` --
  `meta_path_for`/`meta_manifest_for` port `meta_path`/`manifest_for_meta`).
  `PUT`/`DELETE /api/files/<meta>` install/remove it like a manifest (a
  missing file on DELETE is fine) and reply with the playlist fields below,
  for whichever playlist is actually playing; `GET /api/overrides?set=`
  dumps one `.meta`'s overrides, parsed but unresolved. Replies that touch
  the playlist (`/api/reindex`, `/api/config` PUT, a manifest or `.meta`
  install/delete) now carry `overrides`/`hidden` too: how many of the
  playing playlist's entries have one, resolved case-insensitively
  (`opendmd_core::overrides::OverrideSet`), and how many of those are
  `never` (dropped from the simulated playback order -- a "nice to have",
  not modelled: weight, dwell and loop overrides).
- **The schedule and night mode** (`opendmd_core::schedule`): `GET`/`PUT
  /api/schedule` store the JSON verbatim (`""` for the empty schedule, read
  back as `{"rules":[],"night":null}`); `POST /api/wake {"minutes"}`
  suspends it. Evaluated (rules in order, first match wins; night beats
  every rule for `mode`; a wrapped `to <= from` window belongs to the day it
  starts) on every status read, right after a PUT/wake, and once a second in
  the ticker. A rule's `set` overlays the playlist actually playing (`/api/
  config`'s own `set` is untouched); night mode `"off"` freezes the
  simulated player the same way a manual pause does. `time`/`tz` in status
  are the host's local time through `zoneinfo` of the configured `tz_name`
  (falling back to host local time for an unknown zone) -- the mock has no
  real RTC/SNTP to simulate syncing.

# `--strict`

Behave like the current firmware where this mock otherwise differs on
purpose: a `..` escape through the `/*` catch-all is a plain 404 (not 400),
and `/api/stat` and `/api/play` cost a FAT directory scan of the parent
(15 ms + 0.15 ms per entry: ~20 ms in a bucket, ~590 ms in `gifs/Consoles`),
so a UI that stats or plays in a big directory feels it.

# Deliberate deviations from the firmware (per this tool's own spec)

- The real firmware's top-level `/*` catch-all does **not** 400 on a `..`
  escape: `files::card_path` returns `None`, the handler's `if let Some(..)`
  is simply skipped, and it falls through to a plain 404; `/www/*` looks the
  path up in the pack and 404s too. The card routes (`/gifs/*`, `/cache/*`,
  `/sets/*`, `/api/files/*`, `/api/files`) send 400. Without `--strict` the
  catch-all and `/www/*` 400.
- serde's exact "wrong type" error text is not reproduced verbatim (there is
  no serde here); a similarly shaped `"invalid JSON: ..."` message naming the
  offending field is produced instead.

# The now-playing simulation

Status carries `t_ms` (ms since the mock "booted") and, in `now_playing`,
`elapsed_ms` and `frame`: the file's position runs on a simulated clock from
the moment it started (paused: frozen; resumed: shifted by the pause, as the
engine does) through the file's real frame delays — the Graphic Control
Extensions of the prepared copy when there is one, else of the GIF, with
the player's rules (0 -> 100 ms, else >= 20 ms); a copy keeps its GIF's raw
delays, so either gives the same timeline. An ideal device: never late.

`--dwell` (default 5s) is a fixed cadence, independent of the configured
`dwell_min`/`dwell_max` (which are only ever stored and reported, never used
for actual pacing here — real dwell timing depends on each GIF's own loop
length, which this mock does not decode). The active manifest (whichever `set`
resolves to, see above) is read fresh on every rebuild,
filtered by the `platforms` setting (an entry survives if its filename starts
with `<PLATFORM>_`, case-insensitive, for any listed platform — the same
approximation of the real tag parser the task spec calls for), then played in
manifest order or a seeded-random order when `shuffle` is set (seeded, so
repeated rebuilds of the same filtered list are reproducible, not fresh
randomness every time).
"""

import argparse
import datetime
import hashlib
import http.client
import http.server
import json
import math
import os
import random
import re
import socket
import shutil
import sys
import threading
import time
import urllib.parse
from pathlib import Path

# --------------------------------------------------------------------------
# Constants shared with the firmware (config.rs, files.rs, www.rs)
# --------------------------------------------------------------------------

MANIFEST_REL = "filelist.txt"          # config::MANIFEST_PATH, relative to the card root
OBSOLETE_SET = "filelist.odm.txt"      # config::OBSOLETE_ODM_MANIFEST: ignored, refused as a set
MAX_JSON_BODY = 8192                   # http.rs MAX_JSON_BODY
FAILED_REL = "cache/.failed"           # config::BUILDER_FAILED_PATH, card-relative
CACHE_DIR = "cache"                    # the prepared copies (opendmd_core::layout)
LEGACY_DIR = "odm"                     # the old prepared format's tree: ignored
SET_NOT_FOUND = "set not found"        # state::SET_NOT_FOUND
SETS_DIR = "sets"                      # config::SETS_DIR
WWW_PARTITION_SIZE = 1_310_720         # the www partition (partitions.csv)
# Firmware update (docs/ota.md, tools/fw-sign.py): the bundle header and the
# v2 factory partition the image must fit.
OTA_MAGICS = {"4mb": b"ODFW1\0\0\0", "8mb": b"ODFW18mb"}  # opendmd_core::ota, per flash layout
OTA_HEADER_LEN = 140
OTA_SIGNED_LEN = 76
OTA_FACTORY_BYTES = {"4mb": 0x240000, "8mb": 0x300000}   # partitions.csv, partitions-8mb.csv
OTA_BOARDS = {"4mb": "DevKit (4 MB)", "8mb": "8 MB build"}  # opendmd_core::ota::variant_name
RESTART_SECS = 3.0                     # the simulated restart after an update
# `reset_note` for the reasons the mock plays (firmware `Reason::note`).
RESTART_NOTES = {"api_reboot": "restarted on request",
                 "firmware_update": "restarted to install a firmware update",
                 "panel_trial": "restarted to undo a panel timing trial",
                 "card_failure": "card failure"}
MOCK_VERSION = "0.1.0-mock"
# docs/api.md "Security": the API key's alphabet and length, and the routes
# whose body is JSON (a non-empty body must say application/json).
API_KEY_RE = re.compile(r"[A-Za-z0-9._~-]{8,64}")
JSON_ROUTES = {
    ("PUT", "/api/config"), ("PUT", "/api/schedule"), ("POST", "/api/wake"), ("POST", "/api/play"),
    ("POST", "/api/cache/failed"), ("POST", "/api/cache/forget"), ("POST", "/api/follow/test"),
    ("POST", "/api/files/move"), ("POST", "/api/card/format"),
}
FORMAT_SECS = 3.0                      # POST /api/card/format's simulated format
SD_CAPACITY_BYTES = 31_266_439_168     # a plausible "measured" capacity for /api/status

# firmware config::BRIGHTNESS_SAFE_MAX (128: measured on the rig, see there);
# the ceiling's range is opendmd-core brightness::MIN_MAX..=255.
BRIGHTNESS_SAFE_MAX = 144
# firmware config::SAFE_START: /api/status `safe_start_until` is this (uptime
# seconds) until the mock's uptime passes it, then null. The mock has no
# network to wait for, so the window is counted from its start only.
SAFE_START_SECS = 30
MIN_MAX_BRIGHTNESS = 16


def resolve_brightness(brightness, max_b, new_brightness, new_max):
    """opendmd_core::brightness::resolve: (brightness, max) to save, or an
    error message for a 400."""
    if new_max is not None:
        if not (MIN_MAX_BRIGHTNESS <= new_max <= 255):
            return None, f"max_brightness must be {MIN_MAX_BRIGHTNESS}..=255, not {new_max}"
        max_b = new_max
    if new_brightness is not None:
        if new_brightness > max_b:
            return None, (f"brightness {new_brightness} is above the maximum {max_b}; "
                          "raise Maximum brightness first")
        brightness = new_brightness
    return (min(brightness, max_b), max_b), None


# Panel timing (opendmd_core::panel_timing): the same state machine as the
# device's `Tuner`. Times are monotonic seconds; `trial_secs` is the trial's
# length (`--panel-trial`, 60 like the firmware's TRIAL).
PANEL_DEFAULTS = {"clk_phase": True, "latch_blanking": 1, "driver": "shiftreg", "i2s_speed": 8, "min_refresh": 60}
PANEL_DRIVERS = ("shiftreg", "fm6124", "fm6126a", "icn2038s", "mbi5124", "dp3246")
PANEL_BOOT_KEYS = ("driver", "i2s_speed", "min_refresh")
PANEL_TEST_S = 120  # panel_test::LIMIT


def panel_patch(obj):
    """Validate a `panel` object as http.rs + PanelPatch::new do: (patch, None) or (None, error)."""
    if not isinstance(obj, dict):
        return None, "invalid JSON: field 'panel' must be an object or null"
    for k in obj:
        if k not in PANEL_DEFAULTS:
            return None, f"invalid JSON: unknown field `{k}`, expected one of `clk_phase`, `latch_blanking`, `driver`, `i2s_speed`, `min_refresh`"
    out = {}
    for k, v in obj.items():
        if v is None:
            continue
        isint = isinstance(v, int) and not isinstance(v, bool)
        if k == "clk_phase":
            if not isinstance(v, bool):
                return None, "invalid JSON: panel.clk_phase must be a boolean"
            out[k] = v
        elif k == "driver":
            if not isinstance(v, str):
                return None, "invalid JSON: panel.driver must be a string"
            d = v.strip().lower()
            if d not in PANEL_DRIVERS:
                return None, f"panel.driver must be {', '.join(PANEL_DRIVERS)}, not {json.dumps(v)}"
            out[k] = d
        elif not isint:
            return None, f"invalid JSON: panel.{k} must be an integer"
        elif k == "latch_blanking" and not 1 <= v <= 4:
            return None, f"panel.latch_blanking must be 1..=4, not {v}"
        elif k == "i2s_speed" and v not in (8, 16, 20):
            return None, f"panel.i2s_speed must be 8, 16 or 20 (MHz), not {v}"
        elif k == "min_refresh" and not 30 <= v <= 240:
            return None, f"panel.min_refresh must be 30..=240 (Hz), not {v}"
        else:
            out[k] = v
    return out, None


class PanelTuner:
    def __init__(self, trial_secs: float = 60.0):
        self.trial_secs = trial_secs
        self.saved = dict(PANEL_DEFAULTS)
        self.running = dict(PANEL_DEFAULTS)
        self.trial = None      # {"kind": "live"|"boot", "until": monotonic}
        self.staged = None
        self.test = None       # POST /api/panel/test: {"setting", "until"}

    @staticmethod
    def same_boot(a, b):
        return all(a[k] == b[k] for k in PANEL_BOOT_KEYS)

    @staticmethod
    def effective(t):
        return dict(t, clk_phase=t["clk_phase"] or t["driver"] in ("mbi5124", "dp3246"))

    def next_boot(self):
        if self.staged:
            return dict(self.staged)
        return None if self.same_boot(self.saved, self.running) else dict(self.saved)

    def restart_needed(self):
        return self.next_boot() is not None

    @staticmethod
    def test_variants(setting, base):
        """panel_test::variants: A and B for the setting under test."""
        if setting == "clk_phase":
            return dict(base, clk_phase=True), dict(base, clk_phase=False)
        lb = base["latch_blanking"]
        return dict(base), dict(base, latch_blanking=min(lb + 2, 4) if lb < 4 else 2)

    def poll(self):
        """Revert a trial whose time is up; True if that needs a restart."""
        if self.test and time.monotonic() >= self.test["until"]:
            self.test = None
        if self.trial and time.monotonic() >= self.trial["until"]:
            return self.revert()
        return False

    def base(self):
        return dict(self.running) if self.trial else dict(self.staged or self.saved)

    def revert(self):
        self.staged = None
        t, self.trial = self.trial, None
        if t and t["kind"] == "boot":
            return True
        if t:
            self.running.update(clk_phase=self.saved["clk_phase"], latch_blanking=self.saved["latch_blanking"])
        return False

    def try_(self, patch):
        target = dict(self.base(), **patch)
        if self.same_boot(target, self.running):
            kind = self.trial["kind"] if self.trial else "live"
            self.running = target
            self.trial = {"kind": kind, "until": time.monotonic() + self.trial_secs}
            self.staged = None
        else:
            self.staged = target

    def save(self, patch):
        target = dict(PANEL_DEFAULTS) if patch is None else dict(self.base(), **patch)
        self.saved = target
        self.trial = None
        self.staged = None
        self.running.update(clk_phase=target["clk_phase"], latch_blanking=target["latch_blanking"])

    def confirm(self):
        if not self.trial:
            return False
        self.trial = None
        self.saved = dict(self.running)
        return True

    def boot(self):
        """A (simulated) restart: a staged trial runs for this boot only."""
        if self.staged:
            self.running = self.staged
            self.trial = {"kind": "boot", "until": time.monotonic() + self.trial_secs}
        else:
            self.running = dict(self.saved)
            self.trial = None
        self.staged = None

    def fields(self):
        left = None
        if self.trial:
            left = max(0, math.ceil(self.trial["until"] - time.monotonic()))
        return {"panel": dict(self.saved), "panel_now": self.effective(self.running),
                "panel_next": self.next_boot(), "panel_trial_s": left}


DEFAULT_SETTINGS = {
    "wifi_ssid": "MockNet",
    "wifi_pass": "secret",
    "brightness": 96,
    "max_brightness": BRIGHTNESS_SAFE_MAX,
    "dwell_min": 3,
    "dwell_max": 30,
    "shuffle": True,
    "platforms": "",
    "set": "",
    "hostname": "opendmd-mock",
    "tz": "UTC0",
    "tz_name": "UTC",
    "transition": "dissolve",   # config::TRANSITION (opendmd_core::compositor)
    "transition_ms": 400,
    "auto_prepare": "always",   # opendmd_core::auto_prepare: always (default) / idle / off
    "card_speed": "safe",       # config::CARD_SPEED: safe / fast / full
    "follow": False,            # Follow RePlayOS
    "follow_host": "",
    "follow_code": "",          # the NET CONTROL code -- never echoed back
    "attract_set": None,        # None = "the current playlist", else a `set` value
    "attract": "",              # "" (the playlist), "clock" or "text:<message>"
    "look": "colour",           # colour / amber / red / green / blue / white
    "glow": "off",              # off / short / long
    "look_formula": "",         # the Formula look: "" or a formula's @fx:psi?src=.. path
    "look_formula_name": "",    # its name in the web app (display only)
    "look_blend": "over",       # how the web app composed it (display only): over/screen/add/multiply/max/min
    "look_amount": 100,         # and at what amount, 0..=100 (display only)
    "boot_show": "short",       # the start-up show: off / short / full
    "panel_sync": True,         # writes paced to the panel's refresh (no tearing)
    "panel_split": True,        # with it, each frame's bottom half a refresh after the top
    "api_key": "",              # "" = no key asked; never echoed back
}
LOOKS = ("colour", "amber", "red", "green", "blue", "white")
LOOK_BLENDS = ("over", "screen", "add", "multiply", "max", "min")
GLOWS = ("off", "short", "long")
BOOT_SHOWS = ("off", "short", "full")
# The start-up show as a live item (`@boot[?len=short|full|card&..]`,
# opendmd_core::boot): the show once (1300 / 5600 ms) or one loop of the
# status card (16384 ms), in 10 ms frames.
BOOT_MS = {"short": 1300, "full": 5600, "card": 16384}
ATTRACT_MAX = 205               # "text:" + at most 200 characters
GLOW_HELD_BYTES = 12288         # the glow's 12 KB copy

# opendmd_core::compositor::Kind names, and the duration range
TRANSITIONS = ["dissolve", "wipe_left", "wipe_right", "wipe_down", "wipe_up", "blinds", "push", "fade", "none"]
TRANSITION_MS = (20, 2000)

# field -> expected type, for PUT /api/config validation
FIELD_TYPES = {
    "wifi_ssid": "str",
    "wifi_pass": "str",
    "brightness": "u8",
    "max_brightness": "u32",
    "dwell_min": "u64",
    "dwell_max": "u64",
    "shuffle": "bool",
    "platforms": "str",
    "set": "str",
    "hostname": "str",
    "tz": "str",
    "tz_name": "str",
    "transition": "str",
    "transition_ms": "u64",
    "auto_prepare": "str",
    "card_speed": "str",
    "follow": "bool",
    "follow_host": "str",
    "follow_code": "str",
    "attract": "str",
    "look": "str",
    "glow": "str",
    "look_formula": "str",
    "look_formula_name": "str",
    "look_blend": "str",
    "look_amount": "u8",
    "boot_show": "str",
    "panel_sync": "bool",
    "panel_split": "bool",
    "api_key": "str",
    # "attract_set" is not here: it is null-or-string, validated by hand
    # alongside "set" (see h_config_put).
}
AUTO_PREPARE = ("idle", "always", "off")
CARD_SPEED = ("safe", "fast", "full")
# The simulated builder's pace (seconds per file) and its known failures.
AUTO_FULL_S, AUTO_DRIP_S, AUTO_FAIL_EVERY = 2.0, 6.0, 25

SCHEDULE_MAX = 2048            # nvs_config::SCHEDULE_MAX
WAKE_MAX_MINUTES = 24 * 60     # config::WAKE_MAX_MINUTES

STATUS_TEXT = {
    200: "OK",
    202: "Accepted",
    302: "Found",
    400: "Bad Request",
    401: "Unauthorized",
    403: "Forbidden",
    404: "Not Found",
    405: "Method Not Allowed",
    409: "Conflict",
    411: "Length Required",
    413: "Payload Too Large",
    415: "Unsupported Media Type",
    422: "Unprocessable Entity",
    500: "Internal Server Error",
    503: "Service Unavailable",
}

WIFI_SCAN_RESULTS = [
    {"ssid": "MockNet", "rssi": -48, "channel": 6, "auth": "WPA2", "secure": True},
    {"ssid": "Neighbour5G", "rssi": -61, "channel": 36, "auth": "WPA2", "secure": True},
    {"ssid": "CoffeeShop", "rssi": -70, "channel": 11, "auth": "open", "secure": False},
    {"ssid": "IoT_Backhaul", "rssi": -55, "channel": 1, "auth": "WPA3", "secure": True},
    {"ssid": "ApartmentB", "rssi": -82, "channel": 44, "auth": "WPA2", "secure": True},
]


# --------------------------------------------------------------------------
# files.rs equivalents: path handling, mime/cache tables
# --------------------------------------------------------------------------

def percent_decode(s: str) -> str:
    """Port of httpd.rs's percent_decode: '+' -> space, bad escapes pass
    through unchanged, byte-oriented (percent escapes are always ASCII)."""
    b = s.encode("latin-1", "replace")
    out = bytearray()
    i, n = 0, len(b)
    while i < n:
        c = b[i]
        if c == 0x25 and i + 2 < n:  # '%'
            hexpair = b[i + 1:i + 3]
            try:
                out.append(int(hexpair.decode("ascii"), 16))
                i += 3
                continue
            except ValueError:
                out.append(c)
                i += 1
                continue
        elif c == 0x2B:  # '+'
            out.append(0x20)
            i += 1
        else:
            out.append(c)
            i += 1
    return out.decode("utf-8", "replace")


def query_param(query, key):
    if query is None:
        return None
    for pair in query.split("&"):
        if "=" not in pair:
            continue
        k, v = pair.split("=", 1)
        if k == key:
            return percent_decode(v)
    return None


def card_path(rel: str):
    """Port of files::card_path: reject any '..' component. Returns the
    trimmed relative path (possibly ""), or None if it escapes."""
    rel = rel.lstrip("/")
    if "\0" in rel or "\\" in rel:
        return None
    for part in rel.split("/"):
        if part == "..":
            return None
    return rel


def is_root_manifest_name(rel: str) -> bool:
    """files::is_root_manifest_name: `filelist.*`, not its .idx or a temp."""
    name = rel.rsplit("/", 1)[-1]
    return name.startswith("filelist.") and not name.endswith((".idx", ".tmp", ".part"))


def mime_for(rel: str) -> str:
    # Root manifests have whatever extension their owner chose; all text.
    if is_root_manifest_name(rel):
        return "text/plain; charset=utf-8"
    if rel.lower().endswith(".meta"):
        return "text/plain; charset=utf-8"
    ext = Path(rel).suffix.lstrip(".").lower()
    return {
        "html": "text/html; charset=utf-8",
        "htm": "text/html; charset=utf-8",
        "js": "text/javascript; charset=utf-8",
        "mjs": "text/javascript; charset=utf-8",
        "wasm": "application/wasm",
        "css": "text/css; charset=utf-8",
        "json": "application/json",
        "map": "application/json",
        "png": "image/png",
        "gif": "image/gif",
        "jpg": "image/jpeg",
        "jpeg": "image/jpeg",
        "svg": "image/svg+xml",
        "woff2": "font/woff2",
        "ico": "image/x-icon",
        "txt": "text/plain; charset=utf-8",
    }.get(ext, "application/octet-stream")


def cache_for(rel: str) -> str:
    if is_root_manifest_name(rel):
        return "no-cache"
    if rel.lower().endswith(".meta"):
        return "no-cache"
    if "/pkg/" in f"/{rel}":
        return "max-age=3600"
    ext = Path(rel).suffix.lstrip(".").lower()
    if ext in ("html", "htm", "js", "mjs", "json"):
        return "no-cache"
    return "max-age=3600"


def is_manifest_path(rel: str) -> bool:
    """Port of files::is_manifest_path, in root-relative terms."""
    if rel.endswith((".idx", ".tmp", ".part", ".meta")):
        return False
    parts = rel.split("/")
    if len(parts) == 1 and parts[0].startswith("filelist."):
        return True
    if len(parts) == 2 and parts[0] == SETS_DIR:
        return True
    return False


def meta_path_for(manifest_rel: str) -> str:
    """opendmd_core::overrides::meta_path, in root-relative terms: strip a
    '.txt' suffix if there is one, else append '.meta' -- '/filelist.txt' ->
    '/filelist.meta', '/filelist.hall' -> '/filelist.hall.meta',
    '/sets/x.txt' -> '/sets/x.meta'."""
    if manifest_rel.endswith(".txt"):
        return manifest_rel[: -len(".txt")] + ".meta"
    return manifest_rel + ".meta"


def meta_manifest_for(rel: str) -> str | None:
    """Port of files::meta_manifest (+ opendmd_core::overrides::
    manifest_for_meta): the manifest a `.meta` path belongs to, or None if
    `rel` is not a root `filelist*.meta` or a `sets/*.meta`."""
    if not rel.lower().endswith(".meta"):
        return None
    stem = rel[: -len(".meta")]
    if "/" in stem:
        dir_, name = stem.rsplit("/", 1)
    else:
        dir_, name = "", stem
    if not name:
        return None
    in_root = dir_ == ""
    in_sets = dir_ == SETS_DIR
    if not (in_sets or (in_root and name.startswith("filelist"))):
        return None
    if in_sets or name == "filelist":
        return stem + ".txt"
    return stem


PLATFORM_PREFIX_RE = re.compile(r"^[A-Za-z0-9]+_")
# opendmd_core::hostname::valid
HOSTNAME_RE = re.compile(r"[a-z0-9](?:[a-z0-9-]{0,30}[a-z0-9])?")


def platform_list(platforms: str) -> list[str]:
    """The `platforms` setting as uppercased names, blanks dropped."""
    return [p.strip().upper() for p in platforms.split(",") if p.strip()]


def matches_platforms(line: str, platforms: list[str]) -> bool:
    """The filter's approximation (see the module docstring): the file name
    starts with `<PLATFORM>_` for any of them, case-insensitive."""
    name = line.rsplit("/", 1)[-1].upper()
    return any(name.startswith(p + "_") for p in platforms)
TRAILING_NUM_RE = re.compile(r"_\d+$")


def derive_title(manifest_line: str, settings: dict) -> str:
    name = manifest_line.rsplit("/", 1)[-1]
    stem = name.rsplit(".", 1)[0] if "." in name else name
    upper = stem.upper()
    stripped = None
    for p in (p.strip() for p in settings["platforms"].split(",") if p.strip()):
        if upper.startswith(p.upper() + "_"):
            stripped = stem[len(p) + 1:]
            break
    if stripped is None:
        m = PLATFORM_PREFIX_RE.match(stem)
        stripped = stem[m.end():] if m else stem
    stripped = TRAILING_NUM_RE.sub("", stripped)
    return stripped.replace("_", " ")


# --------------------------------------------------------------------------
# Follow RePlayOS: opendmd_core's follow state machine, ported (simplified
# but faithful -- see the Follow class below)
# --------------------------------------------------------------------------

# RePlayOS system id -> (console name, platform tag): the pairs the task
# spec lists. An "arcade_*" id is Arcade/ARCADE; anything else unknown shows
# its id with underscores as spaces and a null platform.
CONSOLE_MAP = {
    "sega_smd": ("Mega Drive", "MD"),
    "nintendo_snes": ("Super Nintendo", "SNES"),
    "nintendo_gb": ("Game Boy", "GB"),
    "nintendo_gbc": ("Game Boy Color", "GBC"),
    "nintendo_gba": ("Game Boy Advance", "GBA"),
    "sega_st": ("Saturn", "SATURN"),
    "nintendo_ds": ("Nintendo DS", "NDS"),
    "nintendo_nes": ("NES", "NES"),
    "sega_sms": ("Master System", "SMS"),
    "nec_pce": ("PC Engine", "PCE"),
    "sony_psx": ("PlayStation", "PS1"),
    "nintendo_n64": ("Nintendo 64", "N64"),
    "sega_gg": ("Game Gear", "GAMEGEAR"),
    "snk_ngp": ("Neo Geo Pocket", "NGP"),
}


def console_for(system_id: str | None):
    """(console name, platform tag) for a RePlayOS system id, or (None,
    None) for an empty one."""
    if not system_id:
        return None, None
    if system_id in CONSOLE_MAP:
        return CONSOLE_MAP[system_id]
    if system_id.startswith("arcade_") or system_id == "arcade":
        return "Arcade", "ARCADE"
    return system_id.replace("_", " "), None


BRACKET_TAGS_RE = re.compile(r"\s*[\(\[][^\)\]]*[\)\]]")


def clean_game_name(name: str) -> str:
    """RePlayOS's game_name with bracketed tags removed: 'Sonic the
    Hedgehog 3 (USA)' -> 'Sonic the Hedgehog 3'."""
    return BRACKET_TAGS_RE.sub("", name or "").strip()


TITLE_STOPWORDS = {"the", "a", "an", "of", "and", "in", "on", "for", "to", "vs", "ii", "iii", "iv", "edition"}
TITLE_TOKEN_RE = re.compile(r"[a-z0-9]+")


def title_tokens(s: str) -> set[str]:
    return {t for t in TITLE_TOKEN_RE.findall(s.lower()) if t not in TITLE_STOPWORDS}


def compute_matches(ctx: "Context", platform: str | None, game_clean: str) -> int | None:
    """A rough stand-in for opendmd_core's `title_score`: entries of the
    playing playlist whose filename has this platform's prefix and whose
    derived title shares at least one non-stopword token with the game
    name."""
    if not platform:
        return None
    want = title_tokens(game_clean)
    if not want:
        return 0
    plat_prefix = platform.upper() + "_"
    with ctx.lock:
        playlist = list(ctx.player.playlist)
        settings = dict(ctx.settings)
    n = 0
    for line in playlist:
        name = line.rsplit("/", 1)[-1].upper()
        if not name.startswith(plat_prefix):
            continue
        if title_tokens(derive_title(line, settings)) & want:
            n += 1
    return n


VIRTUAL_POWER_ON = "@power-on"


# Live effects (`@fx:<name>[?..]`): name -> (title, loop frames at speed 1
# and 50 fps), as opendmd_core::fx; every one plays at 100 fps by default.
FX = {"tunnel": ("Tunnel", 480), "rotozoom": ("Rotozoom", 960),
      "starfield": ("Starfield", 240), "wire": ("Wireframe", 480)}


def live_parse(path: str):
    """A live item (`@clock[?..]`, `@text?msg=..[&..]`, `@fx:<name>[?..]`)
    -> ("clock"|"text"|"fx:<name>", {param: value}) with the values
    percent-decoded, or None for anything else (a text needs a non-empty
    `msg`, an effect a known name)."""
    head, _, query = path.partition("?")
    if head == "@fx:psi" and "src=" in query:
        pass
    elif head.startswith("@fx:") and head[4:] in FX:
        pass
    elif head not in ("@clock", "@text", "@boot"):
        return None
    params: dict[str, str] = {}
    for part in query.split("&") if query else []:
        k, _, v = part.partition("=")
        params[k] = (urllib.parse.unquote_plus if k == "name" else urllib.parse.unquote)(v)
    if head == "@text" and not params.get("msg"):
        return None
    if head == "@fx:psi" and not params.get("src", "").strip():
        return None
    return head[1:], params


def is_live(path: str) -> bool:
    """A path the player draws itself: a live item, `@power-on` or `@card:`."""
    return path == VIRTUAL_POWER_ON or path.startswith("@card:") or live_parse(path) is not None


def live_delays(kind: str, params: dict[str, str]) -> list[int]:
    """Frame delays of a live item: a clock is 1000 ms frames; a text is
    frames of `speed` ms (default 40, 20..1000) over (width + 129) frames,
    width being len*5-1 px (x2 for the big font) or len*4-1 (small); a
    `scroll=0` text that fits in 128 px is one 1000 ms frame."""
    if kind == "clock":
        return [1000]
    if kind == "boot":
        length = params.get("len", "short")
        return [10] * (BOOT_MS.get(length, BOOT_MS["short"]) // 10)
    if kind == "fx:psi":
        # A formula: its `loop N` seconds of 20 ms frames (fps=100: 10, 25: 40).
        delay = {"100": 10, "25": 40}.get(params.get("fps", ""), 20)
        m = re.search(r"(?:^|;|\n)\s*loop\s+([0-9.]+)", params.get("src", ""))
        secs = min(3600.0, max(0.1, float(m.group(1)))) if m else 8.0
        return [delay] * max(1, int(secs * 1000) // delay)
    if kind.startswith("fx:"):
        # A loop: the base frames x 20 / delay / speed (1..4, default 2);
        # 10 ms frames (fps=100, the default), 20 (50) or 40 (25).
        speed = int(params["speed"]) if params.get("speed") in ("1", "2", "3", "4") else 2
        delay = {"50": 20, "25": 40}.get(params.get("fps", ""), 10)
        return [delay] * (FX[kind[3:]][1] * 20 // delay // speed)
    msg = params.get("msg", "")
    font = params.get("font", "big")
    width = (len(msg) * 4 - 1) if font == "small" else (len(msg) * 5 - 1)
    if font not in ("small", "normal"):
        width *= 2
    if params.get("scroll") == "0" and width <= 128:
        return [1000]
    try:
        speed = int(params.get("speed", "40"))
    except ValueError:
        speed = 40
    speed = max(20, min(1000, speed))
    return [speed] * (width + 129)


def virtual_delays(path: str) -> list[int] | None:
    """Frame delays for a virtual "file" (a Follow RePlayOS card): a single
    3000 ms frame for a card (`@card:...`), 35 x 20 ms for the power-on
    effect, a live item's per live_delays; None for a real path."""
    if path == VIRTUAL_POWER_ON:
        return [20] * 35
    if path.startswith("@card:"):
        return [3000]
    live = live_parse(path)
    if live is not None:
        return live_delays(*live)
    return None


def virtual_title(path: str) -> str | None:
    """now_playing.title for a virtual path, or None for a real one."""
    if path == VIRTUAL_POWER_ON:
        return "Power on"
    if path.startswith("@card:"):
        return path[len("@card:"):].split("|", 1)[0]
    live = live_parse(path)
    if live is not None:
        if live[0] == "fx:psi":
            # A playlist entry carries its name (synth::title: 60 characters).
            name = "".join(c for c in live[1].get("name", "") if c.isprintable())[:60]
            return name if name.strip() else "Formula"
        if live[0].startswith("fx:"):
            return FX[live[0][3:]][0]
        if live[0] == "boot":
            return "Startup"
        return "Clock" if live[0] == "clock" else live[1]["msg"]
    return None


def attract_live_path(attract: str) -> str | None:
    """The live item Follow RePlayOS's attract state shows for the config's
    `attract` value (None for "" = the playlist)."""
    if attract == "clock":
        return "@clock"
    if attract.startswith("text:"):
        return "@text?msg=" + urllib.parse.quote(attract[5:].encode(), safe="-_.~")
    if attract.startswith("fx:"):
        return "@" + attract
    return None


def follow_split_host_port(host: str) -> tuple[str, int]:
    """A Follow RePlayOS host as `host[:port]`, port defaulting to 55356."""
    if ":" in host:
        h, _, p = host.rpartition(":")
        try:
            return h, int(p)
        except ValueError:
            return host, 55356
    return host, 55356


def follow_http_get(host: str, port: int, path: str, token: str | None = None, timeout: float = 3.0):
    """One GET against RePlayOS's tiny API; (status, body bytes). Raises on
    a connection failure or timeout -- callers turn that into "no answer"."""
    conn = http.client.HTTPConnection(host, port, timeout=timeout)
    try:
        headers = {"X-RePlay-Token": token} if token is not None else {}
        conn.request("GET", path, headers=headers)
        resp = conn.getresponse()
        return resp.status, resp.read()
    finally:
        conn.close()


class Follow:
    """Follows a real (or mock) RePlayOS box: a background poller, like the
    ticker, that turns `GET get_status` into the `context` fields of
    `/api/status`. Simplified but faithful to opendmd_core's state machine:

    - disabled (follow off, or no host) -> "off": no polling.
    - enabled but never reached / failing -> "unreachable"; leaving
      attract/game/cooldown for it takes 3 failures in a row (a blip should
      not flicker the panel), everything else (off, unreachable itself, a
      fresh enable) goes straight there.
    - reachable and no game loaded -> "attract".
    - reachable and a new game loaded -> "starting": the mock plays its own
      power-on/logo/title-card sequence on "the panel" (see `_run_game`),
      then "game".
    - the loaded game goes away -> "cooldown" for up to 20 s; the same game
      coming back within that window returns to "game" without restarting;
      20 s with nothing loaded -> "attract".
    """

    POLL_OK_S = 2.0
    POLL_BAD_S = 15.0
    TIMEOUT_S = 3.0
    COOLDOWN_S = 20.0
    FAIL_LIMIT = 3

    def __init__(self, ctx: "Context"):
        self.ctx = ctx
        self.lock = threading.Lock()
        self.state = "off"
        self.since = time.monotonic()
        self.fail_count = 0
        self.error: str | None = None
        self.system: str | None = None
        self.platform: str | None = None
        self.console: str | None = None
        self.game: str | None = None
        self.matches: int | None = None
        self.step: str | None = None
        self._loaded_key = None
        self._cooldown_deadline = 0.0
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()

    def stop(self):
        self._stop.set()

    # -- state bookkeeping ------------------------------------------------

    def _set_state(self, state: str, error: str | None = None) -> None:
        with self.lock:
            if state != self.state:
                self.state = state
                self.since = time.monotonic()
            self.error = error

    def _clear_game(self) -> None:
        with self.lock:
            self._loaded_key = None
            self.system = self.platform = self.console = self.game = None
            self.matches = None
            self.step = None

    @staticmethod
    def _is_loaded(body: dict) -> bool:
        view = body.get("view")
        return view == "game_play" or (
            view in ("game_options", "achievements", "leaderboards") and bool(body.get("game_file"))
        )

    # -- the poller ---------------------------------------------------------

    def _run(self) -> None:
        while not self._stop.is_set():
            with self.ctx.lock:
                follow_on = self.ctx.settings["follow"]
                host = self.ctx.settings["follow_host"]
            if not follow_on or not host:
                self._set_state("off")
                self.fail_count = 0
                self._stop.wait(0.5)
                continue
            self._poll_once(host)
            wait_s = self.POLL_BAD_S if self.state == "unreachable" else self.POLL_OK_S
            self._stop.wait(wait_s)

    def _poll_once(self, host: str) -> None:
        h, p = follow_split_host_port(host)
        with self.ctx.lock:
            code = self.ctx.settings["follow_code"]
        try:
            status, data = follow_http_get(h, p, "/api/v1/get_status", token=code, timeout=self.TIMEOUT_S)
        except Exception:  # noqa: BLE001 - any connect/timeout/protocol failure is "no answer"
            self._on_fail("no answer")
            return
        if status == 401:
            self._on_fail("401 (wrong code)")
            return
        if status != 200:
            self._on_fail(f"HTTP {status}")
            return
        try:
            body = json.loads(data.decode("utf-8"))
        except Exception:  # noqa: BLE001 - a malformed reply is "bad reply", not a crash
            self._on_fail("bad reply")
            return
        self.fail_count = 0
        self._on_status(body)

    def _on_fail(self, error: str) -> None:
        with self.lock:
            self.fail_count += 1
            cur_state, fail_n = self.state, self.fail_count
        if cur_state in ("attract", "game", "cooldown") and fail_n < self.FAIL_LIMIT:
            return
        self._set_state("unreachable", error)
        self._clear_game()

    def _on_status(self, body: dict) -> None:
        loaded = self._is_loaded(body)
        key = (body.get("system"), body.get("game_file")) if loaded else None
        now = time.monotonic()
        with self.lock:
            cur_state, cur_key = self.state, self._loaded_key

        if not loaded:
            if cur_state == "game":
                self._set_state("cooldown")
                with self.lock:
                    self._cooldown_deadline = now + self.COOLDOWN_S
                return
            if cur_state == "cooldown":
                with self.lock:
                    deadline = self._cooldown_deadline
                if now >= deadline:
                    self._set_state("attract")
                    self._clear_game()
                return
            if cur_state != "attract":
                self._set_state("attract")
            return

        # A game is loaded.
        if key == cur_key and cur_state in ("game", "cooldown"):
            if cur_state == "cooldown":
                self._set_state("game")  # back within the window -- no restart
            return
        if cur_state == "starting":
            return  # already on it (starting runs to completion below)
        self._run_game(body, key)

    # -- a new game: the mock's own power-on/logo/title sequence -----------

    def _run_game(self, body: dict, key) -> None:
        system = body.get("system") or ""
        console, platform = console_for(system)
        game_clean = clean_game_name(body.get("game_name") or "")
        with self.lock:
            self.system, self.console, self.platform, self.game = system, console, platform, game_clean
            self._loaded_key = key
        self._set_state("starting")
        player = self.ctx.player
        with self.ctx.lock:
            player.virtual_hold = True
        try:
            steps = (
                ("power_on", 0.7, VIRTUAL_POWER_ON),
                ("logo", 3.0, f"@card:{console}|"),
                ("title", 3.0, f"@card:{game_clean}|{console}"),
            )
            for step_name, dur, vpath in steps:
                with self.ctx.lock:
                    if not self.ctx.settings["follow"]:
                        break
                with self.lock:
                    self.step = step_name
                with self.ctx.lock:
                    player.play_now(vpath)
                self._stop.wait(dur)
        finally:
            with self.ctx.lock:
                player.virtual_hold = False
                player.advance()  # hand the panel back to the normal playlist
        with self.lock:
            self.step = None
        matches = compute_matches(self.ctx, platform, game_clean)
        with self.lock:
            self.matches = matches
        self._set_state("game")

    # -- /api/status ---------------------------------------------------------

    def fields(self) -> dict:
        with self.ctx.lock:
            follow_on = self.ctx.settings["follow"]
            host = self.ctx.settings["follow_host"]
        enabled = bool(follow_on and host)
        with self.lock:
            state = self.state
            since = None if state == "off" else int(time.monotonic() - self.since)
            return {
                "source": "replayos",
                "enabled": enabled,
                "state": state,
                "system": self.system,
                "platform": self.platform,
                "console": self.console,
                "game": self.game,
                "matches": self.matches,
                "since": since,
                "step": self.step if state == "starting" else None,
                "error": self.error if state == "unreachable" else None,
            }


def www_version(www_root: Path):
    """The pack header's version: `make web` writes the same string to
    version.txt. Its first line, trimmed, at most 44 characters (the header's
    field); None if missing or blank."""
    try:
        with open(www_root / "version.txt", "rb") as f:
            text = f.read(128).decode("utf-8", "replace")
    except OSError:
        return None
    line = (text.splitlines() or [""])[0].strip()
    return line[:44] or None


def www_info(www_root: Path, pushed: str | None = None) -> dict:
    """GET /api/www, from the files rather than the pack (see the docs);
    `pushed`, the version of the last pack PUT, replaces version.txt's."""
    pack = www_root.parent / "www.pack"
    entries = sum(1 for p in www_root.rglob("*")
                  if p.is_file() and p.suffix != ".gz" and not p.name.startswith(".")
                  and p.relative_to(www_root).as_posix() != "version.txt")
    version = pushed or www_version(www_root)
    ok = (www_root / "index.html").is_file()
    return {"version": version, "bytes": pack.stat().st_size if pack.is_file() else 0,
            "entries": entries, "ok": ok,
            "error": None if ok else "the www partition holds no web app (erased, or never written)"}


def fnv1a32(b: bytes) -> int:
    h = 0x811C9DC5
    for x in b:
        h = ((h ^ x) * 0x01000193) & 0xFFFFFFFF
    return h


def plays_original(reason: str) -> bool:
    """opendmd_core::gifslim::plays_original: a cache/.failed reason that
    means "no copy on purpose, the GIF plays as it is" (more than 256
    colours) -- done, not failed."""
    return reason.lstrip().lower().startswith("more than 256 colo")


def cache_path(path: str):
    """opendmd_core::layout::cache_path: /gifs/Cat/X.gif ->
    /cache/Cat/<xx>/X.gif (xx: FNV-1a of the file name), None for anything
    else."""
    parts = path.lstrip("/").split("/", 1)
    if len(parts) != 2 or parts[0].lower() != "gifs" or not parts[1].lower().endswith(".gif"):
        return None
    d, _, name = parts[1].rpartition("/")
    if len(name) <= 4:
        return None
    xx = f"{fnv1a32(name.encode()) & 0xFF:02x}"
    return f"/cache/{d}/{xx}/{name}" if d else f"/cache/{xx}/{name}"


def gif_delays(b: bytes) -> list[int]:
    """Each frame's delay in ms, by the player's rules (GifSource: a GIF delay
    of 0, or no Graphic Control Extension, is 100 ms; anything else is floored
    at 20 ms). Walks the blocks without decoding a pixel."""
    if b[:3] != b"GIF" or len(b) < 13:
        return []
    flags = b[10]
    i = 13 + (3 * (2 << (flags & 7)) if flags & 0x80 else 0)
    delays, pending = [], 0

    def skip_blocks(i):
        while i < len(b) and b[i]:
            i += b[i] + 1
        return i + 1

    while i < len(b):
        c = b[i]
        i += 1
        if c == 0x21 and i < len(b):
            label = b[i]
            i += 1
            if label == 0xF9 and i + 3 < len(b) and b[i] >= 4:
                pending = b[i + 2] | (b[i + 3] << 8)
            i = skip_blocks(i)
        elif c == 0x2C and i + 9 <= len(b):
            fl = b[i + 8]
            i += 9 + (3 * (2 << (fl & 7)) if fl & 0x80 else 0)
            i = skip_blocks(i + 1)  # past the LZW minimum code size
            delays.append(100 if pending == 0 else max(pending * 10, 20))
            pending = 0
        else:
            break  # 0x3B trailer, or junk
    return delays


def ota_check(data: bytes, pub, variant: str = "4mb") -> str:
    """The bundle's version, or ValueError(reason): opendmd_core::ota's
    checks in the firmware's order (the header, its flash layout against
    this player's `variant`, its signature as it arrives, then whether it
    fits, then the image and its app descriptor) and its messages."""
    from nacl.exceptions import BadSignatureError
    if len(data) < OTA_HEADER_LEN:
        raise ValueError(f"truncated: {len(data)} B, the bundle header is {OTA_HEADER_LEN} B")
    bundle = next((k for k, m in OTA_MAGICS.items() if data[:8] == m), None)
    if bundle is None:
        raise ValueError("not an OpenDMD firmware bundle (bad magic)")
    v = data[8:40].rstrip(b"\0")
    try:
        version = v.decode()
    except UnicodeDecodeError:
        version = ""
    if not version or "\0" in version or not version.isprintable():
        raise ValueError("the bundle's version field is malformed")
    if bundle != variant:
        raise ValueError(f"this update is for the {OTA_BOARDS[bundle]} and this player runs the {OTA_BOARDS[variant]} "
                         f"firmware: use the {OTA_BOARDS[variant]} update file")
    if pub is None:
        raise ValueError("the firmware's public key is invalid")
    try:
        pub.verify(data[:OTA_SIGNED_LEN], data[OTA_SIGNED_LEN:OTA_HEADER_LEN])
    except BadSignatureError:
        raise ValueError("bad signature: not signed by this firmware's key, or the header was changed") from None
    length = int.from_bytes(data[40:44], "little")
    if length > OTA_FACTORY_BYTES[variant]:
        raise ValueError(f"the image is {length} B, the firmware partition is {OTA_FACTORY_BYTES[variant]} B")
    image = data[OTA_HEADER_LEN:]
    if length != len(image):
        raise ValueError(f"length mismatch: the header says {length} B, the image is {len(image)} B")
    if image[:1] != b"\xE9":
        raise ValueError("not an ESP32 app image (the first byte is not 0xE9)")
    if hashlib.sha256(image).digest() != data[44:76]:
        raise ValueError("SHA-256 mismatch: the image is damaged")
    # opendmd_core::ota::check_app_desc: the descriptor's magic word at 0x20,
    # its version at 0x30 ending in " <variant>".
    desc = (image[0x30:0x50].split(b"\0", 1)[0].decode("utf-8", "replace")
            if len(image) >= 0x50 and int.from_bytes(image[0x20:0x24], "little") == 0xABCD5432 else None)
    if desc is None or not desc.endswith(" " + bundle) or len(desc) <= len(bundle) + 1:
        raise ValueError(f"the image inside is not an OpenDMD {bundle} build "
                         f"(its app descriptor says '{desc if desc is not None else 'nothing: no app descriptor'}')")
    return version


def host_port(s: str) -> str:
    """`Origin`/`Host` -> host[:port], lowercased, scheme and `:80` dropped."""
    s = s.strip().lower()
    s = s.split("://", 1)[1] if "://" in s else s
    s = s.split("/", 1)[0]
    return s[:-3] if s.endswith(":80") else s


def enoent(path: str) -> str:
    """Rust's io::Error Display for ENOENT, which the firmware's error
    messages embed verbatim."""
    return f"{path}: No such file or directory (os error 2)"


# --------------------------------------------------------------------------
# Per-GIF overrides ('.meta' files): opendmd_core::overrides, ported
# --------------------------------------------------------------------------

def parse_decimal(s: str) -> float | None:
    """Port of overrides::parse_decimal: a non-negative decimal, ASCII
    digits only, at most 6 int digits and 3 fraction digits, no exponent, no
    sign. `""`/"." are refused (both parts empty)."""
    int_part, sep, frac_part = s.partition(".")
    if not sep:
        frac_part = ""
    if int_part == "" and frac_part == "":
        return None
    if len(int_part) > 6 or len(frac_part) > 3:
        return None
    digits = "0123456789"
    if any(c not in digits for c in int_part) or any(c not in digits for c in frac_part):
        return None

    def to_int(t: str) -> int:
        n = 0
        for c in t:
            n = n * 10 + (ord(c) - 48)
        return n

    milli = to_int(int_part) * 1000 + to_int(frac_part) * (10 ** (3 - len(frac_part)))
    return milli / 1000.0


def num_json(v: float):
    """A decimal value the way overrides::format_decimal/push_decimal would
    print it, as a JSON number: `3` (int) when it is whole, else rounded to
    3 places (`0.5`, `1.25`)."""
    milli = int(max(float(v), 0.0) * 1000.0 + 0.5)
    if milli % 1000 == 0:
        return milli // 1000
    return milli / 1000.0


def apply_override_field(ov: dict, field: str) -> bool:
    """Port of overrides::Override::apply_field. `ov` gains a key only for a
    field it understands and accepts; returns whether it did."""
    key, _, val = field.partition("=")
    if "=" not in field:
        key, val = field.strip(), None
    else:
        key, val = key.strip(), val.strip()
    if key == "never" and val is None:
        ov["never"] = True
        return True
    if key == "w" and val is not None:
        w = parse_decimal(val)
        if w is not None and 0.0 <= w <= 1000.0:
            ov["w"] = w
            return True
        return False
    if key == "dwell" and val is not None:
        if val and val[-1] in "xX":
            k = parse_decimal(val[:-1])
            if k is not None and 0.0 < k <= 100.0:
                ov["dwell_x"] = k
                ov.pop("dwell", None)
                return True
            return False
        if val.isdigit():
            secs = int(val)
            if 1 <= secs <= 86_400:
                ov["dwell"] = secs
                ov.pop("dwell_x", None)
                return True
        return False
    if key == "loops" and val is not None:
        if val.isdigit():
            n = int(val)
            if 1 <= n <= 1000:
                ov["loops"] = n
                return True
        return False
    return False


def parse_meta_line(raw: str):
    """Port of overrides::parse_line: `None` for a blank line, a comment, or
    one whose fields carried nothing valid; else `(path, override_dict)`."""
    line = raw
    if line.startswith("﻿"):
        line = line[1:]
    line = line.rstrip("\n\r \t")
    if not line or line.startswith("#"):
        return None
    parts = line.split("\t")
    path = parts[0].rstrip()
    if not path:
        return None
    ov: dict = {}
    for field in parts[1:]:
        if field.strip():
            apply_override_field(ov, field)
    return (path, ov) if ov else None


def parse_meta_text(text: str) -> dict[str, dict]:
    """Port of overrides::parse_meta: path (exact text) -> override dict, in
    first-seen order; a later line for the same path replaces the whole
    override."""
    out: dict[str, dict] = {}
    for raw in text.split("\n"):
        parsed = parse_meta_line(raw)
        if parsed is not None:
            path, ov = parsed
            out[path] = ov
    return out


def parse_meta_text_for(fs: "CardFS", manifest_rel: str) -> dict[str, dict]:
    """The overrides beside `manifest_rel` (its `.meta`), parsed; `{}` if
    there is none."""
    meta_rel = meta_path_for(manifest_rel)
    p = fs.read_path(meta_rel)
    if p is None:
        return {}
    try:
        text = p.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return {}
    return parse_meta_text(text)


# --------------------------------------------------------------------------
# The schedule: opendmd_core::schedule, ported
# --------------------------------------------------------------------------

def parse_hhmm(s) -> int | None:
    """Port of schedule::parse_hhmm: 'HH:MM' (00:00..23:59) to minutes since
    midnight; '24:00' is accepted as midnight (0)."""
    if not isinstance(s, str) or ":" not in s:
        return None
    h_str, _, m_str = s.strip().partition(":")
    if h_str == "" or len(h_str) > 2 or len(m_str) != 2:
        return None
    if not h_str.isdigit() or not m_str.isdigit():
        return None
    h, m = int(h_str), int(m_str)
    if h == 24 and m == 0:
        return 0
    if 0 <= h <= 23 and 0 <= m <= 59:
        return h * 60 + m
    return None


def format_hhmm(minute: int) -> str:
    m = minute % 1440
    return f"{m // 60:02d}:{m % 60:02d}"


def days_mask(days) -> int | None:
    """Port of schedule::days_mask: ISO weekdays (1=Mon..7=Sun) to a bitmask;
    empty is every day; None if any value is out of range."""
    if not days:
        return 0x7F
    mask = 0
    for d in days:
        if not isinstance(d, int) or isinstance(d, bool) or not (1 <= d <= 7):
            return None
        mask |= 1 << (d - 1)
    return mask


def window_contains(frm: int, to: int, mask: int, weekday: int, minute: int) -> bool:
    """Port of schedule::Window::contains."""
    def on(day: int) -> bool:
        return bool(mask & (1 << (day - 1)))

    yesterday = 7 if weekday == 1 else weekday - 1
    if frm == to:
        return on(weekday)
    if frm < to:
        return on(weekday) and frm <= minute < to
    return (on(weekday) and minute >= frm) or (on(yesterday) and minute < to)


# --------------------------------------------------------------------------
# CardFS: the overlay filesystem
# --------------------------------------------------------------------------

class CardFS:
    """Read-through-overlay, copy-on-write filesystem over a card copy.

    All paths in this API are root-relative POSIX paths with no leading
    slash ("" means the root itself), already validated by `card_path`.
    """

    def __init__(self, root_dir: Path, overlay_dir: Path | None):
        self.root_dir = root_dir
        self.overlay_dir = overlay_dir
        self.tombstones: set[str] = set()
        self.lock = threading.RLock()

    def _write_base(self) -> Path:
        return self.overlay_dir if self.overlay_dir is not None else self.root_dir

    def read_path(self, rel: str) -> Path | None:
        """The filesystem path to read `rel` from (overlay wins), or None."""
        with self.lock:
            if rel in self.tombstones:
                return None
            if self.overlay_dir is not None:
                p = self.overlay_dir / rel if rel else self.overlay_dir
                if p.is_file():
                    return p
            p = self.root_dir / rel if rel else self.root_dir
            return p if p.is_file() else None

    def read_lines(self, rel: str):
        p = self.read_path(rel)
        if p is None:
            return None
        try:
            return p.read_text(encoding="utf-8", errors="replace").splitlines()
        except OSError:
            return None

    def is_dir(self, rel: str) -> bool:
        with self.lock:
            if rel in self.tombstones:
                return False
            if self.overlay_dir is not None:
                p = self.overlay_dir / rel if rel else self.overlay_dir
                if p.is_dir():
                    return True
            p = self.root_dir / rel if rel else self.root_dir
            return p.is_dir()

    def listdir(self, rel_dir: str):
        """Merged (name, is_dir, size_or_None) tuples, or None if the
        directory exists in neither layer."""
        with self.lock:
            bases = [self.root_dir]
            if self.overlay_dir is not None:
                bases.append(self.overlay_dir)
            found_any = False
            order: list[str] = []
            merged: dict[str, tuple[bool, int | None]] = {}
            for base in bases:
                d = base / rel_dir if rel_dir else base
                if not d.is_dir():
                    continue
                found_any = True
                for entry in os.scandir(d):
                    full_rel = f"{rel_dir}/{entry.name}" if rel_dir else entry.name
                    if full_rel in self.tombstones:
                        continue
                    is_dir = entry.is_dir(follow_symlinks=False)
                    size = None
                    if not is_dir:
                        try:
                            size = entry.stat().st_size
                        except OSError:
                            size = None
                    if entry.name not in merged:
                        order.append(entry.name)
                    merged[entry.name] = (is_dir, size)
            if not found_any:
                return None
            return [(name, merged[name][0], merged[name][1]) for name in order]

    def write_bytes(self, rel: str, data: bytes) -> int:
        with self.lock:
            base = self._write_base()
            dest = base / rel
            dest.parent.mkdir(parents=True, exist_ok=True)
            tmp = dest.with_name(dest.name + ".part")
            tmp.write_bytes(data)
            os.replace(tmp, dest)
            self.tombstones.discard(rel)
            # A directory tombstoned when it was emptied comes back with it.
            parts = rel.split("/")
            for i in range(1, len(parts)):
                self.tombstones.discard("/".join(parts[:i]))
            return len(data)

    def cache_files(self) -> list[str]:
        """Card paths of every `.gif` under cache/ in either layer,
        tombstones dropped: the prepared copies."""
        with self.lock:
            found: dict[str, str] = {}
            for base in (self.root_dir, self.overlay_dir):
                if base is None or not (base / CACHE_DIR).is_dir():
                    continue
                for d, _, names in os.walk(base / CACHE_DIR):
                    for n in names:
                        if not n.lower().endswith(".gif"):
                            continue
                        rel = (Path(d) / n).relative_to(base).as_posix()
                        if rel not in self.tombstones:
                            found[rel.lower()] = "/" + rel
            return list(found.values())

    def delete_tree(self, rel: str) -> int:
        """Delete the directory `rel` and everything under it, in both
        layers (root files tombstoned). Returns the files removed."""
        with self.lock:
            files: set[str] = set()
            dirs: set[str] = {rel}
            for base in (self.root_dir, self.overlay_dir):
                if base is None or not (base / rel).is_dir():
                    continue
                for d, dn, names in os.walk(base / rel):
                    drel = Path(d).relative_to(base).as_posix()
                    if drel in self.tombstones:
                        dn[:] = []
                        continue
                    dirs.update(f"{drel}/{x}" for x in dn)
                    files.update(f"{drel}/{n}" for n in names if f"{drel}/{n}" not in self.tombstones)
            if self.overlay_dir is not None and (self.overlay_dir / rel).is_dir():
                shutil.rmtree(self.overlay_dir / rel)
            if self.overlay_dir is None:
                shutil.rmtree(self.root_dir / rel, ignore_errors=True)
            else:
                for r in files | dirs:
                    if (self.root_dir / r).exists():
                        self.tombstones.add(r)
            return len(files)

    def delete(self, rel: str) -> bool:
        """Delete `rel`. Returns True if something was there to delete."""
        with self.lock:
            existed = False
            if self.overlay_dir is not None:
                op = self.overlay_dir / rel
                if op.is_file():
                    op.unlink()
                    existed = True
                rp = self.root_dir / rel
                if rp.exists() and rel not in self.tombstones:
                    self.tombstones.add(rel)
                    existed = True
            else:
                rp = self.root_dir / rel
                if rp.is_file():
                    rp.unlink()
                    existed = True
            return existed


# --------------------------------------------------------------------------
# The now-playing simulation
# --------------------------------------------------------------------------

class Player:
    def __init__(self, ctx: "Context"):
        self.ctx = ctx
        self.boot_time = time.monotonic()
        self.ordinal = 0
        self.gifs_started = 0
        self.over: tuple[str, int] | None = None  # the editor's formula over the GIF, its ordinal
        self.paused = False        # the manual toggle (POST /api/pause)
        self.scheduled_off = False  # night mode "off" (Schedule.evaluate)
        self.virtual_hold = False  # Follow RePlayOS's power-on/logo/title sequence in progress
        self.playlist: list[str] = []
        self.order: list[int] = []
        self.pos = -1
        self.last_advance = 0.0
        self.deadline_misses = 0
        self.worst_lateness_ms = 0
        # As the firmware counts them (opendmd-core frametime): every late
        # frame by cause and lateness; the headline only from 5 ms, steady
        # or at a GIF change.
        self.miss_cause = dict.fromkeys(["open", "read", "decode", "transition", "blit", "log", "other", "wake"], 0)
        self.miss_late = dict.fromkeys(["<5", "<20", "<50", "<100", "<250", "<500", ">=500"], 0)
        self.miss_steady = self.miss_switch = self.worst_steady_ms = 0
        self.manifest = MANIFEST_REL
        self.manifest_last_set: str | None = None
        self.index_state = "building"
        self.played: str | None = None  # an off-playlist POST /api/play
        self.overrides_count = 0
        self.hidden_count = 0
        # The file's own clock: its position is (now or frozen_at) - file_t0,
        # advanced through the real frame delays (see position()). Frozen by
        # a manual pause or by night mode "off" -- either shifts the clocks
        # by however long it lasted, exactly once, on the way out.
        self.file_t0 = time.monotonic()
        self.frozen_at = 0.0
        self.delays: dict[str, list[int]] = {}
        # Recently played (opendmd_core::history): distinct paths, newest
        # first; `recent_at` is the one on screen after Previous,
        # `recent_pending` the entry Previous asked for (-1: none).
        self.recent: list[str] = []
        self.recent_at = 0
        self.recent_pending = -1
        self.rebuild_playlist()

    def _frozen(self) -> bool:
        return self.paused or self.scheduled_off

    def _apply_freeze(self, was_frozen: bool, now_frozen: bool) -> None:
        now = time.monotonic()
        if now_frozen and not was_frozen:
            self.frozen_at = now
        elif was_frozen and not now_frozen:
            shift = now - self.frozen_at
            self.file_t0 += shift
            self.last_advance += shift

    def resolve(self, set_: str) -> str:
        """Settings::manifest_path, root-relative."""
        if not set_:
            return MANIFEST_REL
        if set_.startswith("filelist."):
            return set_
        return f"{SETS_DIR}/{set_}.txt"

    def manifest_rel(self) -> str:
        """The manifest in play, as resolved at the last rebuild."""
        return self.manifest

    def effective_set(self) -> str:
        """The `set` actually playing: a schedule rule's, if one is in
        force, overlaid on the saved config value."""
        rule_set = self.ctx.schedule.rule_set
        return rule_set if rule_set is not None else self.ctx.settings["set"]

    def rebuild_playlist(self):
        self.manifest_last_set = self.effective_set()
        rel = self.manifest = self.resolve(self.manifest_last_set)
        lines = self.ctx.fs.read_lines(rel)
        self.index_state = "ready" if lines is not None else "unavailable"
        base: list[str] = []
        if lines is not None:
            platforms = platform_list(self.ctx.settings["platforms"])
            for raw in lines:
                line = raw.strip()
                if not line:
                    continue
                if platforms and not matches_platforms(line, platforms):
                    continue
                base.append(line)
        # Per-GIF overrides for this playlist, resolved case-insensitively
        # against its entries (opendmd_core::overrides::OverrideSet).
        meta_map = parse_meta_text_for(self.ctx.fs, rel)
        ci: dict[str, dict] = {}
        for path, ov in meta_map.items():
            ci[path.lower()] = ov  # later line (in file order) wins
        overrides_n = 0
        hidden: set[str] = set()
        for b in base:
            ov = ci.get(b.lower())
            if ov is not None:
                overrides_n += 1
                if ov.get("never"):
                    hidden.add(b.lower())
        self.overrides_count = overrides_n
        self.hidden_count = len(hidden)
        # Nice to have: the simulated player skips entries marked `never`.
        filtered = [b for b in base if b.lower() not in hidden]
        self.playlist = filtered
        n = len(filtered)
        self.order = list(range(n))
        if self.ctx.settings["shuffle"] and n > 1:
            random.Random(1234).shuffle(self.order)
        self.pos = -1
        self.ctx.cache.invalidate()
        # A rebuilt engine starts playing immediately, not after a further
        # dwell -- mirror that so a set/platform/manifest change shows a
        # now_playing entry right away instead of an empty one for up to
        # --dwell seconds.
        self.advance()

    def over_on(self) -> bool:
        """The editor's formula is drawn over the file on screen (it goes
        once the playlist moves on: another ordinal)."""
        if self.over is not None and self.over[1] != self.ordinal:
            self.over = None
        return self.over is not None

    def formula_look(self, setting: str):
        """Status `formula_look`: what is drawn over the picture."""
        source = "over_gif" if self.over_on() else "setting" if setting else None
        if source is None:
            return None
        return {"source": source, "held": 20480, "frame_us": 0, "frame_ms": 0}

    def retune(self, path: str):
        """Engine::retune: a formula over the formula on screen, in place
        (the same ordinal, a fresh dwell)."""
        self.played = path
        self.last_advance = time.monotonic()

    def play_now(self, path: str):
        self.played = path
        self.ordinal += 1
        self.gifs_started += 1
        self.last_advance = self.file_t0 = time.monotonic()
        self.started(path)

    def started(self, path: str):
        """History::started: the panel started `path`."""
        stored = recent_stored(path)
        if stored is None:
            return
        pending, self.recent_pending = self.recent_pending, -1
        if 0 <= pending < len(self.recent) and self.recent[pending] == path:
            self.recent_at = pending
            return
        if self.recent_at:
            self.recent.insert(0, self.recent.pop(self.recent_at))
            self.recent_at = 0
        if path in self.recent:
            self.recent.remove(path)
        self.recent.insert(0, path)
        while len(self.recent) > RECENT_LEN or sum(len(recent_stored(p)) + 1 for p in self.recent) > RECENT_BYTES:
            self.recent.pop()

    def back(self) -> str | None:
        """History::back: the entry one older than the one on screen."""
        target = (self.recent_pending if self.recent_pending >= 0 else self.recent_at) + 1
        if target >= len(self.recent):
            return None
        self.recent_pending = target
        return self.recent[target]

    def advance(self):
        self.played = None
        if not self.playlist:
            self.pos = -1
            self.last_advance = time.monotonic()
            return
        # As the engine does when a file will not open: on to the next. A
        # card copy can list a GIF it does not hold, and the Now preview's
        # GET of it would be a 404.
        for _ in range(len(self.order)):
            self.pos = (self.pos + 1) % len(self.order)
            path = self.playlist[self.order[self.pos]]
            if not path.startswith("/") or self.ctx.fs.read_path(path.lstrip("/")) is not None:
                break
        self.ordinal += 1
        self.gifs_started += 1
        self.last_advance = self.file_t0 = time.monotonic()
        self.started(self.current())
        # The real rig: ~1 file start in 10 has its first frame a few ms
        # late (the open); steady play almost never misses, and then by 1 ms.
        if random.random() < 0.1:
            self.record_miss("open", random.randint(0, 7), steady=False)
        if random.random() < 0.02:
            self.record_miss("read", 1, steady=True)

    def record_miss(self, cause: str, late_ms: int, steady: bool):
        self.deadline_misses += 1
        self.worst_lateness_ms = max(self.worst_lateness_ms, late_ms)
        self.miss_cause[cause] += 1
        edges = [5, 20, 50, 100, 250, 500]
        self.miss_late[list(self.miss_late)[sum(late_ms >= e for e in edges)]] += 1
        if steady:
            self.worst_steady_ms = max(self.worst_steady_ms, late_ms)
            self.miss_steady += late_ms >= 5
        else:
            self.miss_switch += late_ms >= 5

    def misses_json(self) -> dict:
        return {"steady": self.miss_steady, "switch": self.miss_switch, "threshold_ms": 5, "window_ms": 500,
                "worst_steady_ms": self.worst_steady_ms, "by_cause": dict(self.miss_cause), "late_ms": dict(self.miss_late)}

    def set_paused(self, paused: bool):
        """As the engine does: a pause freezes the file's position, and
        resuming shifts its clocks (and the dwell's) by however long it
        lasted."""
        was = self._frozen()
        self.paused = paused
        self._apply_freeze(was, self._frozen())

    def set_scheduled_off(self, off: bool):
        """Night mode "off": the simulated player pauses (a separate reason
        from the manual toggle, so either can come and go independently)."""
        if off == self.scheduled_off:
            return
        was = self._frozen()
        self.scheduled_off = off
        self._apply_freeze(was, self._frozen())

    def frame_delays(self, path: str) -> list[int]:
        """The frame delays of what the device would play for `path`: its
        prepared copy if there is one, else the file. Cached per path."""
        if path not in self.delays:
            vd = virtual_delays(path)
            if vd is not None:
                self.delays[path] = vd
                return vd
            out = []
            for cand in (cache_path(path), path):
                rel = card_path(cand) if cand else None
                f = self.ctx.fs.read_path(rel) if rel else None
                if f is None:
                    continue
                b = f.read_bytes()
                out = gif_delays(b)
                if out:
                    break
            self.delays[path] = out
        return self.delays[path]

    def position(self):
        """(elapsed_ms, frame): the file's position in its own timeline and
        the 1-based number of the frame that covers it, loops included — an
        ideal device, never late."""
        cur = self.current()
        delays = self.frame_delays(cur) if cur else []
        loop = sum(delays)
        if not loop:
            return 0, 0
        at = self.frozen_at if self._frozen() else time.monotonic()
        ms = int((at - self.file_t0) * 1000)
        loops, rem = divmod(ms, loop)
        idx = 0
        while idx < len(delays) and rem >= delays[idx]:
            rem -= delays[idx]
            idx += 1
        return ms, loops * len(delays) + idx + 1

    def tick(self):
        if self._frozen() or not self.playlist or self.virtual_hold:
            return
        if time.monotonic() - self.last_advance >= self.ctx.dwell:
            self.advance()

    def current(self):
        if self.played:
            return self.played
        if not self.playlist or self.pos < 0:
            return None
        return self.playlist[self.order[self.pos]]

    def reset_boot(self):
        self.boot_time = time.monotonic()
        self.recent, self.recent_at, self.recent_pending = [], 0, -1
        self.ordinal = 0
        self.gifs_started = 0
        self.pos = -1
        self.last_advance = 0.0
        self.advance()


RECENT_LEN, RECENT_BYTES, RECENT_MAX_ENTRY = 16, 640, 200


def recent_stored(path: str) -> str | None:
    """opendmd_core::history's stored form of a path (/gifs/X.gif -> X), or
    None if it is not recorded (Follow's power-on and cards, a relative path,
    an entry over 200 B)."""
    if not path or "\n" in path or path == "@power-on" or path.startswith("@card:"):
        return None
    s = path
    if path.startswith("/gifs/") and path.endswith(".gif") and len(path) > 10 and path[6] not in "/@":
        s = path[6:-4]
    elif path[0] not in "/@":
        return None
    return s if len(s) <= RECENT_MAX_ENTRY else None


def heap_numbers(player: Player):
    t = time.monotonic() - player.boot_time
    free = 57000 + int(1200 * math.sin(t / 4.0)) + random.randint(-150, 150)
    floor = 50000 + int(400 * math.sin(t / 7.0 + 1.0))
    idf_min = 45000
    return max(free, 1000), max(floor, 1000), idf_min


# --------------------------------------------------------------------------
# The schedule: time/night mode simulation (opendmd_core::schedule, sched.rs)
# --------------------------------------------------------------------------

class Schedule:
    """Rules + night mode, evaluated against the mock's own clock.

    `doc` is the stored schedule exactly as PUT it (or `None` for "no
    schedule", read back as the empty form) -- what GET /api/schedule
    returns verbatim. `evaluate()` (called on every status read, from the
    ticker, and right after a PUT/wake/tz change) turns it into `mode`,
    `mode_until`, `rule_idx`/`rule_set`/`until` and, through
    `Player.effective_set`/`Player.set_scheduled_off`, the playlist actually
    playing and whether it is paused for the night.
    """

    EMPTY = {"rules": [], "night": None}

    def __init__(self, ctx: "Context"):
        self.ctx = ctx
        self.doc: dict | None = None
        self.wake_until_mono: float | None = None
        self.mode = "normal"
        self.mode_until: str | None = None
        self.rule_idx: int | None = None
        self.rule_set: str | None = None
        self.until: str | None = None
        self._norm_rules: list = []
        self._norm_night: dict | None = None

    def get_doc(self) -> dict:
        return self.doc if self.doc is not None else dict(self.EMPTY)

    def local_now(self):
        """Host local time via zoneinfo of the configured tz_name, or plain
        host local time if it is not a known zone."""
        tz_name = self.ctx.settings.get("tz_name") or "UTC"
        try:
            from zoneinfo import ZoneInfo
            return datetime.datetime.now(ZoneInfo(tz_name))
        except Exception:
            return datetime.datetime.now()

    # -- validation + storage ------------------------------------------

    def _validate(self, body):
        """Returns None (and leaves `self.doc` untouched -- the caller
        stores it) on success, or (status, error-body-or-message)."""
        if not isinstance(body, dict):
            return (400, "invalid JSON: expected a JSON object")
        rules_in = body.get("rules", [])
        if rules_in is None:
            rules_in = []
        if not isinstance(rules_in, list):
            return (400, "invalid JSON: 'rules' must be an array")
        if len(rules_in) > 16:
            return (400, f"at most 16 rules (got {len(rules_in)})")
        norm_rules = []
        for i, r in enumerate(rules_in):
            what = f"rule {i + 1}"
            if not isinstance(r, dict):
                return (400, f"{what}: must be an object")
            mask = days_mask(r.get("days") or [])
            if mask is None:
                return (400, f"{what}: days are 1 (Mon) to 7 (Sun)")
            frm = parse_hhmm(r.get("from"))
            if frm is None:
                return (400, f"{what}: from '{r.get('from')}' is not HH:MM")
            to = parse_hhmm(r.get("to"))
            if to is None:
                return (400, f"{what}: to '{r.get('to')}' is not HH:MM")
            set_name = r.get("set")
            if set_name is not None:
                if (not isinstance(set_name, str) or any(c in set_name for c in "/\\\0")
                        or ".." in set_name or set_name.lower() == OBSOLETE_SET.lower()):
                    return (400, f"{what}: '{set_name}' is not a playlist name")
            brightness = r.get("brightness")
            if brightness is not None and (
                not isinstance(brightness, int) or isinstance(brightness, bool) or not (0 <= brightness <= 255)
            ):
                return (400, f"{what}: brightness must be 0..255")
            norm_rules.append({"days": mask, "from_min": frm, "to_min": to, "set": set_name, "raw": r})

        night_in = body.get("night")
        night_norm = None
        if night_in is not None:
            if not isinstance(night_in, dict):
                return (400, "invalid JSON: 'night' must be an object or null")
            nfrm = parse_hhmm(night_in.get("from"))
            if nfrm is None:
                return (400, f"night: from '{night_in.get('from')}' is not HH:MM")
            nto = parse_hhmm(night_in.get("to"))
            if nto is None:
                return (400, f"night: to '{night_in.get('to')}' is not HH:MM")
            mode = night_in.get("mode")
            if mode not in ("off", "dim"):
                return (400, f"night: mode '{mode}' is not off or dim")
            nb = night_in.get("brightness")
            if nb is not None and (not isinstance(nb, int) or isinstance(nb, bool) or not (0 <= nb <= 255)):
                return (400, "night: brightness must be 0..255")
            night_norm = {"from_min": nfrm, "to_min": nto, "mode": mode, "raw": night_in}

        # Whether each rule's playlist opens is checked after everything
        # else is well-formed (sched::missing_sets): every missing one is
        # named, not just the first.
        missing = []
        for rn in norm_rules:
            name = rn["set"]
            if name and name not in missing and self.ctx.fs.read_lines(self.ctx.player.resolve(name)) is None:
                missing.append(name)
        if missing:
            return (404, {"error": f"set not found: {', '.join(missing)}"})

        self._norm_rules = norm_rules
        self._norm_night = night_norm
        return None

    def validate_and_set(self, body):
        err = self._validate(body)
        if err is not None:
            return err
        rules_raw = [r["raw"] for r in self._norm_rules]
        night_raw = self._norm_night["raw"] if self._norm_night is not None else None
        self.doc = None if (not rules_raw and night_raw is None) else {"rules": rules_raw, "night": night_raw}
        self.evaluate()
        return None

    def wake(self, minutes: int):
        self.wake_until_mono = None if minutes <= 0 else time.monotonic() + minutes * 60
        self.evaluate()

    def wake_until_str(self) -> str | None:
        if self.wake_until_mono is None:
            return None
        remaining = self.wake_until_mono - time.monotonic()
        if remaining <= 0:
            return None
        until_dt = self.local_now() + datetime.timedelta(seconds=remaining)
        return until_dt.strftime("%H:%M")

    # -- evaluation ------------------------------------------------------

    def evaluate(self):
        with self.ctx.lock:
            now = self.local_now()
            if self.wake_until_mono is not None and time.monotonic() >= self.wake_until_mono:
                self.wake_until_mono = None
            woken = self.wake_until_mono is not None

            weekday = now.isoweekday()
            minute = now.hour * 60 + now.minute
            doc = self.doc
            rules = (doc or {}).get("rules") or []
            night = (doc or {}).get("night")

            rule_idx = rule_set = until = None
            if not woken:
                for i, r in enumerate(rules):
                    mask = days_mask(r.get("days") or [])
                    frm, to = parse_hhmm(r.get("from")), parse_hhmm(r.get("to"))
                    if mask is None or frm is None or to is None:
                        continue  # a stored rule should always be valid; be defensive
                    if window_contains(frm, to, mask, weekday, minute):
                        rule_idx = i
                        rule_set = r.get("set")
                        until = format_hhmm(to)
                        break

            mode, mode_until = "normal", None
            if not woken and night is not None:
                nfrm, nto = parse_hhmm(night.get("from")), parse_hhmm(night.get("to"))
                if nfrm is not None and nto is not None and window_contains(nfrm, nto, 0x7F, weekday, minute):
                    mode = night.get("mode") or "normal"
                    mode_until = format_hhmm(nto)

            self.mode = mode
            self.mode_until = mode_until
            self.rule_idx = rule_idx
            self.rule_set = rule_set
            self.until = until
            self.ctx.player.set_scheduled_off(mode == "off")
            if self.ctx.player.effective_set() != self.ctx.player.manifest_last_set:
                self.ctx.player.rebuild_playlist()

    def status_fields(self) -> dict:
        return {
            "mode": self.mode,
            "mode_until": self.mode_until,
            "wake_until": self.wake_until_str(),
            "schedule": {"rule": self.rule_idx, "set": self.rule_set, "until": self.until},
        }


# --------------------------------------------------------------------------
# Shared context
# --------------------------------------------------------------------------

class Cache:
    """cache.rs: counted off the request path, only once someone has asked,
    and again after every change."""

    def __init__(self, ctx: "Context"):
        self.ctx = ctx
        self.lock = threading.Lock()
        self.gen = 0
        self.done_gen = -1
        self.prepared = None
        self.counting = False
        self.count_ms = 0
        self.requested = False
        self.missing: list[str] = []  # the playlist's entries with no copy, as last counted

    def invalidate(self):
        self.ctx.auto.recount()
        with self.lock:
            self.gen += 1
            start = self.requested and not self.counting
            if self.requested:
                self.counting = True
        if start:
            self._spawn()

    def snapshot(self):
        with self.lock:
            self.requested = True
            start = self.done_gen != self.gen and not self.counting
            if start:
                self.counting = True
            snap = (self.prepared, self.counting, self.count_ms)
        if start:
            self._spawn()
        return snap

    def _spawn(self):
        threading.Thread(target=self._run, daemon=True).start()

    def _run(self):
        while True:
            time.sleep(0.6)  # the device debounces; a short one here
            with self.lock:
                gen = self.gen
            t0 = time.monotonic()
            with self.ctx.lock:
                playlist = list(self.ctx.player.playlist)
            have = {p.lower() for p in self.ctx.fs.cache_files()}
            # GIFs that play as they are (no copy on purpose: more than 256
            # colours, recorded in cache/.failed) count as prepared.
            have |= {cache_path(g).lower() for g in self.ctx.plays_as_is() if cache_path(g)}
            n = 0
            missing = []
            for p in playlist:
                if p.startswith("@"):  # a live item: layout::classify says never
                    continue
                if p.lower().startswith("/cache/"):
                    n += 1
                    continue
                c = cache_path(p)
                if c and c.lower() in have:
                    n += 1
                else:
                    missing.append(p)
            with self.lock:
                if gen != self.gen:
                    continue
                self.prepared, self.done_gen, self.counting = n, gen, False
                self.missing = missing
                self.count_ms = int((time.monotonic() - t0) * 1000)
                return


class AutoPrepare:
    """The device's own preparing ("auto prepare"), simulated. It writes
    nothing: files it "prepares" are a count (`done`) on top of the real
    counted copies, walked through the last count's missing list, and every
    recount (Cache.invalidate: an upload, forget, delete or rebuild) drops it
    along with the pass's failures -- so `prepared` is the real count unless
    the simulation has run since the last change. Callers hold ctx.lock."""

    def __init__(self, ctx: "Context"):
        self.ctx = ctx
        self.done = 0        # simulated copies in this pass
        self.failed = 0      # every AUTO_FAIL_EVERY-th file "cannot be prepared"
        self.session = 0     # prepared since boot (kept across recounts)
        self.progress = 0.0  # fraction of the file in hand
        self.measured = {"full": False, "drip": False}
        self.last = time.monotonic()

    def recount(self):
        with self.ctx.lock:
            self.done = self.failed = 0
            self.progress = 0.0

    def _missing(self):
        c = self.ctx.cache
        with c.lock:
            return (list(c.missing) if c.prepared is not None else None), c.prepared

    def situation(self):
        """(state, reason, policy, remaining missing paths or None)."""
        mode = self.ctx.settings["auto_prepare"]
        missing, _ = self._missing()
        if mode == "off":
            return "idle", "off", None, missing
        if not self.ctx.player.playlist:
            return "idle", "no playlist", None, missing
        if missing is None:
            return "idle", None, None, None
        left = missing[self.done + self.failed:]
        if not left:
            return "done", None, None, left
        if self.ctx.player._frozen():
            return "running", None, "full", left
        if mode == "always":
            return "running", None, "drip", left
        return "paused", "playing", None, left

    def tick(self):
        now = time.monotonic()
        dt, self.last = now - self.last, now
        state, _, policy, left = self.situation()
        if state != "running":
            return
        self.progress += dt / (AUTO_FULL_S if policy == "full" else AUTO_DRIP_S)
        while self.progress >= 1 and left:
            self.progress -= 1
            left = left[1:]
            if (self.done + self.failed + 1) % AUTO_FAIL_EVERY == 0:
                self.failed += 1
            else:
                self.done += 1
                self.session += 1
                self.measured[policy] = True
        if not left:
            self.progress = 0.0

    def prepared(self):
        _, n = self._missing()
        return None if n is None else n + self.done

    def fields(self) -> dict:
        state, reason, policy, left = self.situation()
        full = AUTO_FULL_S if self.measured["full"] else None
        return {
            "mode": self.ctx.settings["auto_prepare"],
            "state": state,
            "reason": reason,
            "policy": policy,
            "session": self.session,
            "failed": self.failed,
            "avg_full_s": full,
            "avg_drip_s": AUTO_DRIP_S if self.measured["drip"] else None,
            "eta_s": int(len(left) * full) if full and left is not None else None,
            "current": left[0] if state == "running" and left else None,
        }

    def status_fields(self) -> dict:
        return {"state": self.situation()[0], "prepared": self.prepared(),
                "total": len(self.ctx.player.playlist)}


class Context:
    def __init__(self, fs: CardFS, www_root: Path, dwell: float, strict: bool):
        self.fs = fs
        self.www_root = www_root
        self.dwell = dwell
        self.strict = strict
        self.settings = dict(DEFAULT_SETTINGS)
        self.panel = PanelTuner()
        self.lock = threading.RLock()
        self.auto = AutoPrepare(self)
        self.cache = Cache(self)
        self.schedule = Schedule(self)
        self.player = Player(self)
        self.follow = Follow(self)
        # The card's health (firmware `sdhealth.rs`): the clock it runs at,
        # the slow-clock fallback, card-failure restarts. Driven by the same
        # test hooks as the device: POST /api/reboot?sd_io_fail=3 plays a
        # card-failure restart (the next "boot" is at 20 MHz, reset_note
        # "card failure"); ?sd_fallback=clear forgets it.
        self.sd = {"clock_khz": 40000, "fallback": False, "failures": 0, "reboots": 0,
                   "probe": None, "slowdowns": 0, "recoveries": 0}
        # Whether this card is recorded slow (what `card_speed: fast` reads;
        # `--sd slowed` and a card-failure restart set it).
        self.sd_slow_record = False
        # Every restart's reason (firmware `restarts.rs`), newest first, at
        # most 8; a power-on starts it. `restart()` adds one.
        self.restarts = [{"reason": "power_on", "reset": "poweron",
                          "uptime_s": None, "heap_free": None, "largest": None}]
        # Card file serving as the tests see it (`GET /mock/io`, mock only):
        # requests for card files (/gifs/, /cache/) in flight and at most,
        # bytes actually sent (a client that hangs up stops the count), and
        # an optional pace, `kbps`, which sends in the firmware's 2 KB
        # pieces (`config::IO_CHUNK`) at that rate, so a reader that stops
        # early (the web app's thumbnails) is measured as on a device.
        self.io = {"kbps": 0, "inflight": 0, "max": 0, "gets": 0, "bytes": 0, "cut": 0,
                   "r503": 0, "fail503": 0, "fail500": 0, "drop": 0}
        # The device's stream task (firmware/src/stream.rs): card and web app
        # files, and the listings (/api/files, /api/cache/list, /failed), go
        # out one at a time with QUEUE_DEPTH (2) waiting; a web app file or
        # a playlist that finds the queue full waits up to QUEUE_WAIT (2 s)
        # for a place, a listing, GIF or copy not at all, then gets 503 +
        # Retry-After: 1.
        self.streamq = {"cond": threading.Condition(), "busy": False, "waiting": 0}
        self.schedule.evaluate()  # picks up the playlist rebuilt just above
        # Firmware update: the running version, the updater's last line, the
        # simulated restart's end (monotonic), the Ed25519 key (VerifyKey).
        self.version = MOCK_VERSION
        self.fw_last = None
        self.fw_log = None
        self.down_until = 0.0
        self.ota_pub = None
        # --variant / --flash-size: the flash layout this player runs, and its chip.
        self.variant = "4mb"
        self.flash_size = 4 << 20
        # --sd: "ok", "unreadable" (mounts, filesystem unreadable) or "none".
        self.sd_state = "ok"
        # Names other devices answer to (`--taken`), and how long a check
        # listens when no one does (`--name-query`).
        self.taken: dict[str, str] = {}
        self.name_query = 1.2
        # POST /api/card/format: when the simulated format ends (monotonic,
        # 0: none running), and why the last one failed.
        self.format_until = 0.0
        self.format_error = None
        # PUT /api/www's last `staged` (None: no PUT yet).
        self.www_staged = None
        # The header version of the last pack PUT (None: version.txt's).
        self.www_pushed = None

    def restart(self, reason: str):
        """A restart for `reason` (firmware `crate::restart`): its entry
        first in `restarts`, paired with a software reset. Caller holds
        the lock."""
        self.restarts = [{"reason": reason, "reset": "software",
                          "uptime_s": int(time.monotonic() - self.player.boot_time),
                          "heap_free": 98000, "largest": 45056}] + self.restarts[:7]

    def plays_as_is(self) -> list[str]:
        """The GIFs cache/.failed says play as they are (plays_original)."""
        fp = self.fs.read_path(FAILED_REL)
        if fp is None:
            return []
        out = []
        for line in fp.read_text(encoding="utf-8", errors="replace").split("\n"):
            gif, _, why = line.partition("\t")
            if gif.strip() and plays_original(why):
                out.append(gif.strip())
        return out


def apply_card_speed(ctx: "Context") -> None:
    """The clock and fallback `card_speed` gives the card: safe 20 MHz, full
    40 MHz, fast 20 MHz when this card is recorded slow, else 40 MHz."""
    mode = ctx.settings["card_speed"]
    slow = mode == "safe" or (mode == "fast" and ctx.sd_slow_record)
    ctx.sd.update(clock_khz=20000 if slow else 40000, fallback=(mode == "fast" and ctx.sd_slow_record))


SD_UNREADABLE = "card not readable: it needs FAT32; format it from the web app"


def sd_fields(ctx: "Context") -> dict:
    """/api/status's `sd`, per --sd (and a POST /api/card/format)."""
    if ctx.format_until and time.monotonic() >= ctx.format_until:
        # The format is over: the card reads now (an empty FAT32 card on a
        # device; the mock keeps serving --root).
        ctx.format_until = 0.0
        ctx.sd_state = "ok"
    base = {"mounted": True, "readable": True, "capacity_bytes": SD_CAPACITY_BYTES, "error": None, "attempts": 1,
            "formatting": bool(ctx.format_until), "format_error": ctx.format_error}
    if ctx.sd_state == "unreadable":
        base.update(readable=False, capacity_bytes=63_864_569_856, error=SD_UNREADABLE)
    elif ctx.sd_state == "none":
        base.update(mounted=False, readable=False, capacity_bytes=0, attempts=2,
                    error="no card, or card init failed: ESP_ERR_TIMEOUT")
    out = {**base, **ctx.sd}
    if ctx.sd_state == "none":
        out["clock_khz"] = None
    return out


def ticker_loop(ctx: Context, stop: threading.Event):
    while not stop.wait(0.5):
        with ctx.lock:
            ctx.schedule.evaluate()
            ctx.player.tick()
            ctx.auto.tick()


# --------------------------------------------------------------------------
# HTTP handler (card mode)
# --------------------------------------------------------------------------

def make_handler(ctx: Context, quiet: bool):
    def name_check(name: str) -> str | None:
        """Who answers to <name>.local (its IP), or None: as the network
        worker's mDNS query, quick when someone does, the whole wait when
        no one does."""
        by = ctx.taken.get(name)
        time.sleep(0.05 if by else ctx.name_query)
        return by

    class Handler(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"
        server_version = "OpenDMD-mock/0.1"

        # -- plumbing ---------------------------------------------------

        def log_message(self, fmt, *args):
            pass

        def _run(self, method):
            self._resp_status = None
            self._resp_bytes = 0
            t0 = time.monotonic()
            try:
                self.route(method)
            except (BrokenPipeError, ConnectionResetError):
                pass
            except Exception as e:  # noqa: BLE001 - mirror the firmware's catch-all 500
                sys.stderr.write(f"mock-device: unhandled error on {method} {self.path}: {e!r}\n")
                if self._resp_status is None:
                    self.send_error_json(500, "internal error")
            if not quiet:
                dt_ms = (time.monotonic() - t0) * 1000.0
                print(
                    f"{method:6s} {self.path:60s} {self._resp_status} "
                    f"{self._resp_bytes}B {dt_ms:.1f}ms",
                    file=sys.stderr,
                )

        def do_GET(self):
            self._run("GET")

        def do_POST(self):
            self._run("POST")

        def do_PUT(self):
            self._run("PUT")

        def do_DELETE(self):
            self._run("DELETE")

        # -- response helpers --------------------------------------------

        def send_json(self, code: int, obj, headers: dict | None = None) -> None:
            body = json.dumps(obj, separators=(",", ":")).encode("utf-8")
            self.send_response(code, STATUS_TEXT.get(code, ""))
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            for k, v in (headers or {}).items():
                self.send_header(k, v)
            self.end_headers()
            if self.command != "HEAD":
                self.wfile.write(body)
            self._resp_status = code
            self._resp_bytes = len(body)

        def send_error_json(self, code: int, message: str, headers: dict | None = None) -> None:
            self.send_json(code, {"error": message}, headers)

        def streamed(self, serve, wait: float = 2.0) -> None:
            """Run `serve` as the stream task would: alone, behind at most two
            waiting; a queue full for `wait` s (2 for a file, 0 for a
            listing: stream::hand_off_within) is a 503 with Retry-After: 1."""
            q = ctx.streamq
            with q["cond"]:
                deadline = time.monotonic() + wait
                while q["waiting"] >= 2:
                    left = deadline - time.monotonic()
                    if left <= 0:
                        with ctx.lock:
                            ctx.io["r503"] += 1
                        self.send_error_json(503, "the stream queue is full; retry shortly", {"Retry-After": "1"})
                        return
                    q["cond"].wait(left)
                q["waiting"] += 1
                while q["busy"]:
                    q["cond"].wait()
                q["waiting"] -= 1
                q["busy"] = True
            try:
                serve()
            finally:
                with q["cond"]:
                    q["busy"] = False
                    q["cond"].notify_all()

        def drain(self) -> None:
            """Read and drop the request body, so the connection stays usable."""
            remaining = int(self.headers.get("Content-Length") or 0)
            while remaining > 0:
                chunk = self.rfile.read(min(65536, remaining))
                if not chunk:
                    break
                remaining -= len(chunk)

        def refuse(self, code: int, message: str, headers: dict | None = None) -> None:
            self.drain()
            self.send_error_json(code, message, headers)

        def guard(self, method: str, path: str, query) -> bool:
            """docs/api.md "Security" and "Ground rules": True when the request
            was refused (and answered) here. Every handler runs behind it."""
            if time.monotonic() < ctx.down_until and path.startswith("/api/"):
                self.drain()
                self.send_error_json(503, "restarting", {"Retry-After": "1"})
                return True
            origin = self.headers.get("Origin")
            if method in ("POST", "PUT", "DELETE") and origin is not None \
                    and host_port(origin) != host_port(self.headers.get("Host") or ""):
                self.refuse(403, "cross-origin request refused")
                return True
            key = ctx.settings["api_key"]
            if key and (path.startswith("/api/") or (method == "GET" and (
                    path.startswith(("/gifs/", "/cache/", "/sets/", "/filelist"))))):
                auth = self.headers.get("Authorization") or ""
                if auth != "Bearer " + key and query_param(query, "key") != key:
                    self.refuse(401, "API key required", {"WWW-Authenticate": "Bearer"})
                    return True
            if (method, path) in JSON_ROUTES and int(self.headers.get("Content-Length") or 0) > 0 \
                    and not (self.headers.get("Content-Type") or "").lower().startswith("application/json"):
                self.refuse(415, "Content-Type must be application/json")
                return True
            return False

        def stream_file(self, actual: Path, logical_rel: str, gzipped: bool) -> None:
            try:
                size = actual.stat().st_size
            except OSError:
                self.send_error_json(404, logical_rel)
                return
            self.send_response(200, "OK")
            self.send_header("Content-Type", mime_for(logical_rel))
            if gzipped:
                self.send_header("Content-Encoding", "gzip")
            self.send_header("Cache-Control", cache_for(logical_rel))
            self.send_header("Content-Length", str(size))
            self.end_headers()
            if self.command != "HEAD":
                with open(actual, "rb") as f:
                    shutil.copyfileobj(f, self.wfile)
            self._resp_status = 200
            self._resp_bytes = size

        def serve_www_rel(self, rel: str) -> bool:
            out = []
            base = ctx.www_root / rel if rel else ctx.www_root
            gz = base.with_name(base.name + ".gz") if rel else None
            if base.is_dir() or not (base.is_file() or (gz is not None and gz.is_file())):
                return False
            self.streamed(lambda: out.append(self.serve_www_now(rel)))
            return True

        def serve_www_now(self, rel: str) -> bool:
            """Serve `rel` (root-relative, no leading slash) out of --www,
            preferring `<rel>.gz` unconditionally. Returns False on a miss
            (nothing sent yet)."""
            base = ctx.www_root / rel if rel else ctx.www_root
            if base.is_dir():
                return False
            gz = base.with_name(base.name + ".gz") if rel else None
            if gz is not None and gz.is_file():
                self.stream_file(gz, rel, gzipped=True)
                return True
            if base.is_file():
                self.stream_file(base, rel, gzipped=False)
                return True
            return False

        def read_body(self) -> bytes:
            length = int(self.headers.get("Content-Length") or 0)
            return self.rfile.read(length) if length else b""

        # -- routing -------------------------------------------------------

        def route(self, method: str):
            path, _, query = self.path.partition("?")
            query = query if "?" in self.path else None
            if self.guard(method, path, query):
                return

            def m_exact(p):
                return lambda cur: cur == p

            def m_prefix(p):
                return lambda cur: cur.startswith(p)

            routes = [
                (m_exact("/"), {"GET": self.h_root}),
                (m_exact("/setup"), {"GET": self.h_setup}),
                (m_exact("/api/status"), {"GET": self.h_status}),
                (m_exact("/api/skip"), {"POST": self.h_skip}),
                (m_exact("/api/pause"), {"POST": self.h_pause}),
                (m_exact("/api/config"), {"GET": self.h_config_get, "PUT": self.h_config_put}),
                (m_exact("/api/reindex"), {"POST": self.h_reindex}),
                (m_exact("/api/overrides"), {"GET": self.h_overrides_get}),
                (m_exact("/api/schedule"), {"GET": self.h_schedule_get, "PUT": self.h_schedule_put}),
                (m_exact("/api/wake"), {"POST": self.h_wake}),
                (m_exact("/api/card/format"), {"POST": self.h_card_format}),
                (m_exact("/api/card/full-speed"), {"POST": self.h_card_full_speed}),
                (m_exact("/api/follow/test"), {"POST": self.h_follow_test}),
                (m_exact("/api/play"), {"POST": self.h_play}),
                (m_exact("/api/panel/test"), {"POST": self.h_panel_test}),
                (m_exact("/api/stat"), {"GET": self.h_stat}),
                (m_exact("/api/cache"), {"GET": self.h_cache}),
                (m_exact("/api/cache/list"), {"GET": lambda p: self.streamed(lambda: self.h_cache_list(p), 0)}),
                (m_exact("/api/cache/forget"), {"POST": self.h_cache_forget}),
                (m_exact("/api/cache/failed"), {"GET": lambda p: self.streamed(lambda: self.h_cache_failed_get(p), 0), "POST": self.h_cache_failed_post,
                                                "DELETE": self.h_cache_failed_delete}),
                # Listings go out from the stream task too (firmware http.rs).
                (m_exact("/api/files"), {"GET": lambda p: self.streamed(lambda: self.h_files_list(p), 0)}),
                (m_exact("/api/wifi/scan"), {"GET": self.h_wifi_scan}),
                (m_exact("/api/name-check"), {"GET": self.h_name_check}),
                (m_exact("/api/www"), {"GET": self.h_www_get, "PUT": self.h_www_put}),
                (m_exact("/api/reboot"), {"POST": self.h_reboot}),
                (m_exact("/api/firmware"), {"GET": self.h_firmware_get, "PUT": self.h_firmware_put}),
                (m_exact("/api/manifest"), {"PUT": self.h_manifest_put}),
                (m_prefix("/api/sets/"), {"PUT": self.h_sets_put}),
                (m_exact("/api/files/move"), {"POST": self.h_files_move}),
                (m_prefix("/api/files/"), {"PUT": self.h_files_put, "DELETE": self.h_files_delete}),
                (m_prefix("/filelist"), {"GET": self.h_filelist}),
                (m_prefix("/gifs/"), {"GET": self.h_serve_under}),
                (m_prefix("/sets/"), {"GET": self.h_serve_under}),
                (m_prefix("/cache/"), {"GET": self.h_serve_under}),
                (m_prefix("/www/"), {"GET": self.h_www}),
                (m_exact("/mock/io"), {"GET": self.h_mock_io}),
                (lambda cur: True, {"GET": self.h_catchall}),
            ]

            matched_any = False
            for matcher, methods in routes:
                if matcher(path):
                    matched_any = True
                    handler = methods.get(method)
                    if handler is not None:
                        handler(path)
                        return
            # "/*" (GET) matches literally everything, so matched_any is always
            # True in practice -- mirroring ESP-IDF's httpd, where a URI that
            # matches some registered pattern under a *different* method than
            # the one requested answers 405, not 404. See the module docstring.
            if matched_any:
                self.send_error_json(405, "method not allowed")
            else:
                self.send_error_json(404, path)

        # -- static file handlers -------------------------------------------

        def h_root(self, path):
            if not self.serve_www_rel("index.html"):
                self.send_error_json(404, "index.html")

        def h_setup(self, path):
            """The firmware's embedded setup page (build.rs renders it)."""
            html = (Path(__file__).resolve().parent.parent / "firmware" / "web" / "setup.html").read_text()
            body = (html.replace("{{ version }}", ctx.version).replace("{{ variant }}", ctx.variant)
                    .replace("{{ variant_name }}", OTA_BOARDS[ctx.variant]).encode())
            self.send_response(200, "OK")
            self.send_header("Content-Type", "text/html")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def h_www(self, path):
            rel_raw = path[len("/www/"):]
            rel = card_path(percent_decode(rel_raw))
            if rel is None:
                if ctx.strict:  # the firmware looks it up in the pack: not there
                    self.send_error_json(404, rel_raw)
                    return
                self.send_error_json(400, "path escapes the card")
                return
            if not self.serve_www_rel(rel):
                self.send_error_json(404, rel)

        def h_catchall(self, path):
            rel_full = percent_decode(path)
            rel = card_path(rel_full)
            if rel is None:
                if ctx.strict:  # the firmware's `if let Some(..)` falls through
                    self.send_error_json(404, rel_full)
                    return
                self.send_error_json(400, "path escapes the card")
                return
            # A Follow RePlayOS virtual "file" (`@power-on`, `@card:...`) is
            # drawn by the page's own preview engine, never fetched; a 404
            # here means the UI slipped and asked for one anyway.
            if rel.startswith("@"):
                self.send_error_json(404, rel_full)
                return
            if not self.serve_www_rel(rel):
                self.send_error_json(404, rel_full)

        def h_filelist(self, path):
            rel_full = percent_decode(path)
            name = rel_full.lstrip("/")
            if "/" in name:
                self.send_error_json(404, rel_full)
                return
            rel = card_path(name)
            if rel is None:
                self.send_error_json(400, "path escapes the card")
                return

            def serve():
                fpath = ctx.fs.read_path(rel)
                if fpath is None:
                    self.send_error_json(404, rel_full)
                    return
                self.stream_file(fpath, rel, gzipped=False)
            self.streamed(serve)

        def stream_paced(self, fpath: Path, rel: str, io: dict) -> None:
            data = fpath.read_bytes()
            self.send_response(200, "OK")
            self.send_header("Content-Type", mime_for(rel))
            self.send_header("Cache-Control", cache_for(rel))
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            sent = 0
            try:
                while sent < len(data):
                    piece = data[sent:sent + 2048]
                    self.wfile.write(piece)
                    self.wfile.flush()
                    sent += len(piece)
                    with ctx.lock:
                        io["bytes"] += len(piece)
                    time.sleep(len(piece) / (io["kbps"] * 1024))
            except (BrokenPipeError, ConnectionResetError):
                with ctx.lock:
                    io["cut"] += 1
                self.close_connection = True
            self._resp_status = 200
            self._resp_bytes = sent

        # GET /mock/io (mock only, not a firmware route): the card-file
        # counters above; `?reset=1` zeroes them, `&kbps=N` sets the pace
        # (0: as fast as the machine goes); `&fail503=N` answers the next N
        # card-file GETs 503, `&fail500=N` 500, `&drop=N` closes them
        # unanswered (a browser may retry those by itself).
        def h_mock_io(self, path):
            _, _, query = self.path.partition("?")
            io = ctx.io
            with ctx.lock:
                out = dict(io)
                if query_param(query, "reset") == "1":
                    io.update(max=io["inflight"], gets=0, bytes=0, cut=0, r503=0)
                for key in ("kbps", "fail503", "fail500", "drop"):
                    k = query_param(query, key)
                    if k is not None:
                        io[key] = max(0, int(k))
            self.send_json(200, out)

        def h_serve_under(self, path):
            rel_full = percent_decode(path)
            rel = card_path(rel_full)
            if rel is None:
                self.send_error_json(400, "path escapes the card")
                return
            if rel.startswith("@") or "/@" in f"/{rel}":
                self.send_error_json(404, rel_full)
                return
            io = ctx.io
            with ctx.lock:
                card = rel_full.startswith(("/gifs/", "/cache/"))
                fail = card and next((k for k in ("fail503", "fail500", "drop") if io[k]), None)
                if fail:
                    io[fail] -= 1
            if fail == "fail503":  # injected (GET /mock/io?fail503=N)
                with ctx.lock:
                    io["r503"] += 1
                self.send_error_json(503, "busy (injected)", {"Retry-After": "1"})
                return
            if fail == "fail500":  # injected: a card error, say
                self.send_error_json(500, "card read failed (injected)")
                return
            if fail == "drop":  # injected: the connection closes, no answer
                self.close_connection = True
                self.connection.shutdown(socket.SHUT_RDWR)
                return
            # GIFs and copies do not wait for a place (http.rs serve_under); playlists 2 s.
            self.streamed(lambda: self.serve_card(fpath_rel=rel, rel_full=rel_full), 0 if card else 2.0)

        def serve_card(self, fpath_rel: str, rel_full: str) -> None:
            rel, io = fpath_rel, ctx.io
            with ctx.lock:
                io["gets"] += 1
                io["inflight"] += 1
                io["max"] = max(io["max"], io["inflight"])
            try:
                fpath = ctx.fs.read_path(rel)
                if fpath is None:
                    self.send_error_json(404, rel_full)
                    return
                if not io["kbps"]:
                    self.stream_file(fpath, rel, gzipped=False)
                    with ctx.lock:
                        io["bytes"] += self._resp_bytes
                    return
                self.stream_paced(fpath, rel, io)
            finally:
                with ctx.lock:
                    io["inflight"] -= 1

        # -- JSON API handlers -----------------------------------------------

        def h_status(self, path):
            with ctx.lock:
                player = ctx.player
                ctx.schedule.evaluate()
                player.tick()
                cur = player.current()
                free, floor, idf_min = heap_numbers(player)
                elapsed_ms, frame = player.position()
                now_local = ctx.schedule.local_now()
                time_synced_s = min(int(time.monotonic() - player.boot_time), 600)
                obj = {
                    "version": ctx.version,
                    "variant": ctx.variant,
                    "uptime_secs": int(time.monotonic() - player.boot_time),
                    "t_ms": int((time.monotonic() - player.boot_time) * 1000),
                    "ip": "127.0.0.1",
                    "wifi": "sta",
                    "hostname": ctx.settings["hostname"],
                    "mdns": ctx.settings["hostname"],
                    "now_playing": {
                        "ordinal": player.ordinal,
                        "path": cur if cur else "",
                        "title": (virtual_title(cur) if cur and cur.startswith("@") else derive_title(cur, ctx.settings)) if cur else "",
                        "elapsed_ms": elapsed_ms,
                        "frame": frame,
                    },
                    "paused": player.paused,
                    "entries": len(player.playlist),
                    "manifest": "/sdcard/" + player.manifest_rel(),
                    "index_state": player.index_state,
                    "www_version": ctx.www_pushed or www_version(ctx.www_root),
                    "www": {**www_info(ctx.www_root, ctx.www_pushed), "staged": ctx.www_staged},
                    "heap": {"free": free, "floor": floor, "idf_min": idf_min, "transitions_cut": 0},
                    "refresh_hz": 100,
                    "deadline_misses": player.deadline_misses,
                    "worst_lateness_ms": player.worst_lateness_ms,
                    "misses": player.misses_json(),
                    "gifs_started": player.gifs_started,
                    "sd": sd_fields(ctx),
                    "reset_reason": ctx.restarts[0]["reset"],
                    "reset_note": RESTART_NOTES.get(ctx.restarts[0]["reason"]),
                    "restarts": ctx.restarts,
                    "overrides": player.overrides_count,
                    "hidden": player.hidden_count,
                    "mode": ctx.schedule.mode,
                    "mode_until": ctx.schedule.mode_until,
                    "time": now_local.strftime("%H:%M"),
                    "tz": ctx.settings["tz_name"],
                    "time_synced_s": time_synced_s,
                    "wake_until": ctx.schedule.wake_until_str(),
                    "schedule": {"rule": ctx.schedule.rule_idx, "set": ctx.schedule.rule_set, "until": ctx.schedule.until},
                    "cache": ctx.auto.status_fields(),
                    "context": ctx.follow.fields(),
                    "look": ctx.settings["look"],
                    "glow": ctx.settings["glow"],
                    "glow_held": GLOW_HELD_BYTES if ctx.settings["glow"] != "off" and not player._frozen() else 0,
                    "formula_look": player.formula_look(ctx.settings["look_formula"]),
                    "attract_live": None,
                    "max_brightness": ctx.settings["max_brightness"],
                    "brightness_safe_max": BRIGHTNESS_SAFE_MAX,
                    "safe_start_until": SAFE_START_SECS
                    if time.monotonic() - player.boot_time < SAFE_START_SECS else None,
                    # The panel's scan (no seams counted: there is no panel).
                    "scan": {"row_us": 499, "sync": ctx.settings["panel_sync"], "blits": 0, "torn": 0,
                             "in_pair": 0, "waited": 0, "wait_us": 0, "wait_max_us": 0, "gave_up": 0,
                             "split_on": ctx.settings["panel_sync"] and ctx.settings["panel_split"],
                             "split": 0, "split_ok": 0, "unsplit": 0, "spin_us": 0,
                             "stretched": 0, "late": 0, "retimed": 0},
                }
                if ctx.settings["follow"] and ctx.follow.state == "attract":
                    obj["attract_live"] = attract_live_path(ctx.settings["attract"])
                _, _, query = self.path.partition("?")
                if query_param(query, "recent") == "1":
                    # Recently played, right after `paused` as the firmware puts it.
                    items = list(obj.items())
                    k = list(obj).index("paused") + 1
                    obj = dict(items[:k] + [("recent", list(player.recent)), ("recent_at", player.recent_at)] + items[k:])
                if ctx.sd_state != "ok":
                    # No card the player can use: nothing plays (the boot
                    # task never hands an unreadable card over).
                    obj["now_playing"].update(path="", title="", elapsed_ms=0, frame=0)
                    obj.update(entries=0, index_state="unavailable")
            self.send_json(200, obj)

        def h_skip(self, path):
            _, _, query = self.path.partition("?")
            if query_param(query, "back") != "1":
                with ctx.lock:
                    ctx.player.advance()
                self.send_json(200, {"ok": True})
                return
            # Previous: one step back through Recently played.
            with ctx.lock:
                player = ctx.player
                if not player.playlist:
                    self.send_error_json(503, "no playlist is running")
                    return
                p = player.back()
                if p is None:
                    self.send_error_json(404, "nothing earlier")
                    return
                rel = card_path(p) if p.startswith("/") else None
                if rel is not None and ctx.fs.read_path(rel) is None:
                    player.recent_pending = -1
                    self.send_error_json(404, enoent(p))
                    return
                player.play_now(p)
            self.send_json(200, {"ok": True})

        def h_pause(self, path):
            with ctx.lock:
                ctx.player.set_paused(not ctx.player.paused)
                paused = ctx.player.paused
            self.send_json(200, {"ok": True, "paused": paused})

        def h_config_get(self, path):
            with ctx.lock:
                s = ctx.settings
                obj = {
                    "wifi_ssid": s["wifi_ssid"],
                    "wifi_pass_set": bool(s["wifi_pass"]),
                    "brightness": s["brightness"],
                    "max_brightness": s["max_brightness"],
                    "dwell_min": s["dwell_min"],
                    "dwell_max": s["dwell_max"],
                    "shuffle": s["shuffle"],
                    "platforms": s["platforms"],
                    "set": s["set"],
                    "hostname": s["hostname"],
                    "tz": s["tz"],
                    "tz_name": s["tz_name"],
                    "transition": s["transition"],
                    "transition_ms": s["transition_ms"],
                    "auto_prepare": s["auto_prepare"],
                    "card_speed": s["card_speed"],
                    "follow": s["follow"],
                    "follow_host": s["follow_host"],
                    "follow_code_set": bool(s["follow_code"]),
                    "attract_set": s["attract_set"],
                    "attract": s["attract"],
                    "look": s["look"],
                    "glow": s["glow"],
                    "look_formula": s["look_formula"],
                    "look_formula_name": s["look_formula_name"],
                    "look_blend": s["look_blend"],
                    "look_amount": s["look_amount"],
                    "boot_show": s["boot_show"],
                    "panel_sync": s["panel_sync"],
                    "panel_split": s["panel_split"],
                    "api_key_set": bool(s["api_key"]),
                }
                self.panel_poll()
                obj.update(ctx.panel.fields())
            self.send_json(200, obj)

        def panel_poll(self):
            """A trial whose time is up reverts; a boot trial by a simulated
            restart. Caller holds ctx.lock."""
            if ctx.panel.poll():
                self.panel_restart()

        def panel_restart(self):
            """A restart to undo a boot trial: as POST /api/reboot."""
            ctx.restart("panel_trial")
            ctx.panel.boot()
            ctx.player.reset_boot()
            ctx.down_until = time.monotonic() + RESTART_SECS
            ctx.player.boot_time += RESTART_SECS

        def playlist_fields(self, index, index_ms) -> dict:
            """`"entries","index","index_ms","overrides","hidden"` for a
            reply that touched the playlist -- always the one actually
            playing (http.rs `applied_json`). Caller holds ctx.lock."""
            return {
                "entries": len(ctx.player.playlist),
                "index": index,
                "index_ms": index_ms,
                "overrides": ctx.player.overrides_count,
                "hidden": ctx.player.hidden_count,
            }

        def json_body(self) -> bytes | None:
            """http.rs json_body: None means a 413 has been sent."""
            length = int(self.headers.get("Content-Length") or 0)
            if length > MAX_JSON_BODY:
                self.rfile.read(length)  # keep the connection usable
                self.send_error_json(413, f"body is {length} B, the limit is {MAX_JSON_BODY} B")
                return None
            return self.read_body()

        def dir_scan(self, rel: str) -> None:
            """--strict: the FAT scan of `rel`'s parent that one open/stat costs."""
            if ctx.strict:
                parent = rel.rsplit("/", 1)[0] if "/" in rel else ""
                n = 0
                for base in (ctx.fs.root_dir, ctx.fs.overlay_dir):
                    d = base / parent if base is not None else None
                    if d is not None and d.is_dir():
                        n += len(os.listdir(d))
                time.sleep(0.015 + 0.00015 * n)

        def h_config_put(self, path):
            raw = self.json_body()
            if raw is None:
                return
            try:
                patch = json.loads(raw) if raw else {}
            except json.JSONDecodeError as e:
                self.send_error_json(400, f"invalid JSON: {e}")
                return
            if not isinstance(patch, dict):
                self.send_error_json(400, "invalid JSON: expected a JSON object")
                return

            def type_ok(v, kind):
                if kind == "str":
                    return isinstance(v, str)
                if kind == "bool":
                    return isinstance(v, bool)
                if kind == "u8":
                    return isinstance(v, int) and not isinstance(v, bool) and 0 <= v <= 255
                if kind in ("u64", "u32"):
                    return isinstance(v, int) and not isinstance(v, bool) and v >= 0
                return False

            with ctx.lock:
                before = dict(ctx.settings)
                next_settings = dict(ctx.settings)
                for field, kind in FIELD_TYPES.items():
                    if field not in patch:
                        continue
                    v = patch[field]
                    if not type_ok(v, kind):
                        self.send_error_json(
                            400, f"invalid JSON: field '{field}' must be a {kind}"
                        )
                        return
                    if field == "set" and (any(c in v for c in "/\\\0") or ".." in v):
                        self.send_error_json(400, "set must be a single path segment")
                        return
                    if field == "tz" and (not v.strip() or len(v.strip()) > 64 or "\0" in v):
                        self.send_error_json(
                            400, "tz must be a POSIX TZ string, e.g. GMT0IST,M3.5.0/1,M10.5.0"
                        )
                        return
                    if field == "tz_name" and len(v) > 64:
                        self.send_error_json(400, "tz_name is at most 64 characters")
                        return
                    if field == "transition":
                        # Kind::from_name: case-insensitive, '-' for '_', "wipe" = wipe_left
                        v = v.strip().lower().replace("-", "_")
                        v = "wipe_left" if v == "wipe" else v
                        if v not in TRANSITIONS:
                            self.send_error_json(
                                400, "transition must be dissolve, wipe_left, wipe_right, wipe_down, wipe_up, blinds, push, fade or none")
                            return
                    if field == "transition_ms" and not TRANSITION_MS[0] <= v <= TRANSITION_MS[1]:
                        self.send_error_json(400, f"transition_ms must be {TRANSITION_MS[0]}..={TRANSITION_MS[1]}")
                        return
                    if field == "look":
                        v = "colour" if v == "color" else v
                        if v not in LOOKS:
                            self.send_error_json(400, "look must be colour, amber, red, green, blue or white")
                            return
                    if field == "glow" and v not in GLOWS:
                        self.send_error_json(400, "glow must be off, short or long")
                        return
                    if field == "look_formula" and v and not v.startswith("@fx:psi?"):
                        self.send_error_json(400, "look_formula must be \"\" or a formula (@fx:psi?src=…)")
                        return
                    if field == "look_formula" and len(v.encode()) > 2048:
                        self.send_error_json(413, f"this formula is too long for the panel ({len(v.encode())} B as sent; the limit is 2048 B)")
                        return
                    if field == "look_formula_name" and (len(v) > 60 or any(ord(c) < 32 for c in v)):
                        self.send_error_json(400, "look_formula_name must be one line of at most 60 characters")
                        return
                    if field == "look_blend" and v not in LOOK_BLENDS:
                        self.send_error_json(400, "look_blend must be over, screen, add, multiply, max or min")
                        return
                    if field == "look_amount" and not 0 <= v <= 100:
                        self.send_error_json(400, "look_amount must be 0..=100")
                        return
                    if field == "api_key" and v != "" and not API_KEY_RE.fullmatch(v):
                        self.send_error_json(400, "api_key must be 8..64 of A-Z a-z 0-9 . _ ~ - (or \"\" for none)")
                        return
                    if field == "boot_show" and v not in BOOT_SHOWS:
                        self.send_error_json(400, "boot_show must be off, short or full")
                        return
                    if field == "attract" and not (
                        v in ("", "clock")
                        or (v.startswith("fx:") and len(v.encode()) <= ATTRACT_MAX and live_parse("@" + v))
                        or (v.startswith("text:") and v[5:].strip() and len(v.encode()) <= ATTRACT_MAX
                            and not any(ord(c) < 32 or 0x7F <= ord(c) < 0xA0 for c in v))
                    ):
                        self.send_error_json(
                            400, 'attract must be "" (the playlist), "clock", "text:<message>" (at most 200 characters, one line) or "fx:<effect>[?options]"')
                        return
                    if field == "auto_prepare" and v not in AUTO_PREPARE:
                        self.send_error_json(400, "auto_prepare must be idle, always or off")
                        return
                    if field == "card_speed" and v not in CARD_SPEED:
                        self.send_error_json(400, "card_speed must be safe, fast or full")
                        return
                    if field == "follow_host":
                        v = v.strip()
                        if len(v) > 64 or any(c in v for c in "/ \0?#@"):
                            self.send_error_json(400, "follow_host must be a host name or address, optionally :port")
                            return
                    if field == "follow_code" and (len(v) > 64 or any(c in v for c in "\0\r\n")):
                        self.send_error_json(400, "follow_code must be at most 64 characters, with no control characters")
                        return
                    if field == "hostname":
                        v = v.strip().lower()
                        if v != "" and not HOSTNAME_RE.fullmatch(v):
                            self.send_error_json(400, "hostname must be 1-32 lowercase letters, digits and hyphens, not starting or ending with a hyphen")
                            return
                        if v != "":
                            next_settings["hostname"] = v
                    elif field == "tz":
                        next_settings["tz"] = v.strip()
                    elif field in ("dwell_min", "dwell_max"):
                        next_settings[field] = max(1, v)
                    else:
                        next_settings[field] = v
                # A new look sent without how it was composed is drawn as it is.
                if "look_formula" in patch and "look_blend" not in patch and "look_amount" not in patch:
                    next_settings["look_blend"], next_settings["look_amount"] = "over", 100

                # "attract_set" is null-or-string, so it is not in
                # FIELD_TYPES; validated the same way as "set" below.
                if "attract_set" in patch:
                    av = patch["attract_set"]
                    if av is not None and not isinstance(av, str):
                        self.send_error_json(400, "invalid JSON: field 'attract_set' must be a string or null")
                        return
                    if isinstance(av, str) and (any(c in av for c in "/\\\0") or ".." in av):
                        self.send_error_json(400, "attract_set must be a single path segment")
                        return
                    next_settings["attract_set"] = av

                if next_settings["dwell_max"] < next_settings["dwell_min"]:
                    self.send_error_json(400, "dwell_max must be >= dwell_min")
                    return

                # A name another device answers to is refused, nothing
                # saved (http.rs asks the network worker last, before the
                # main task's checks).
                if next_settings["hostname"] != before["hostname"]:
                    by = name_check(next_settings["hostname"])
                    if by:
                        name = next_settings["hostname"]
                        self.send_json(409, {"error": f"name taken: {name}.local already answers from {by}",
                                             "name": name, "by": by})
                        return

                # WiFi takes effect after a reboot; a new hostname at once
                # (status `hostname`/`mdns` read the saved one).
                reboot_required = (
                    next_settings["wifi_ssid"] != before["wifi_ssid"]
                    or next_settings["wifi_pass"] != before["wifi_pass"]
                )
                needs_reindex = (
                    next_settings["platforms"] != before["platforms"]
                    or next_settings["set"] != before["set"]
                    or next_settings["shuffle"] != before["shuffle"]
                )

                # A new set must open before anything is saved ("" always
                # passes: it is filelist.txt). The obsolete one never does.
                if next_settings["set"] != before["set"] and next_settings["set"].lower() == OBSOLETE_SET:
                    self.send_error_json(404, SET_NOT_FOUND)
                    return
                if next_settings["set"] != before["set"] and next_settings["set"]:
                    if ctx.fs.read_path(ctx.player.resolve(next_settings["set"])) is None:
                        self.send_error_json(404, SET_NOT_FOUND)
                        return

                # A platform filter that would leave the playlist empty is
                # refused, nothing saved (main.rs builds it first).
                plats = platform_list(next_settings["platforms"])
                if needs_reindex and plats and (
                        next_settings["platforms"] != before["platforms"] or next_settings["set"] != before["set"]):
                    rule_set = ctx.schedule.rule_set
                    rel = ctx.player.resolve(rule_set if rule_set is not None else next_settings["set"])
                    lines = ctx.fs.read_lines(rel)
                    if lines is not None and not any(matches_platforms(l.strip(), plats) for l in lines if l.strip()):
                        self.send_error_json(400, f'no GIFs in the playlist match the platform filter "{next_settings["platforms"].strip()}"; nothing was saved')
                        return

                pair, err = resolve_brightness(
                    before["brightness"], before["max_brightness"],
                    patch.get("brightness"), patch.get("max_brightness"))
                if err:
                    self.send_error_json(400, err)
                    return
                next_settings["brightness"], next_settings["max_brightness"] = pair

                # Same rule for "attract_set" when it changes and is non-empty.
                if next_settings["attract_set"] != before["attract_set"] and next_settings["attract_set"]:
                    if next_settings["attract_set"].lower() == OBSOLETE_SET:
                        self.send_error_json(404, SET_NOT_FOUND)
                        return
                    if ctx.fs.read_path(ctx.player.resolve(next_settings["attract_set"])) is None:
                        self.send_error_json(404, SET_NOT_FOUND)
                        return

                # Panel timing (http.rs: an object is a trial, null the
                # defaults, panel_confirm keeps / undoes).
                panel_op = None
                if "panel" in patch or "panel_confirm" in patch:
                    pc = patch.get("panel_confirm")
                    if pc is not None and not isinstance(pc, bool):
                        self.send_error_json(400, "invalid JSON: field 'panel_confirm' must be a boolean")
                        return
                    if "panel" in patch and patch["panel"] is None:
                        panel_op = ("save", None)
                    elif "panel" in patch:
                        if pc is False:
                            self.send_error_json(400, "panel_confirm false undoes a trial: send it without panel")
                            return
                        pp, err = panel_patch(patch["panel"])
                        if err:
                            self.send_error_json(400, err)
                            return
                        panel_op = ("save", pp) if pc else ("try", pp)
                    elif pc is not None:
                        panel_op = ("confirm",) if pc else ("revert",)
                    self.panel_poll()
                    if panel_op == ("confirm",) and not ctx.panel.trial:
                        self.send_error_json(409, "no panel trial is running (it may have ended)")
                        return

                # A settings change ends the panel test (main.rs, before
                # anything applies).
                ctx.panel.test = None
                # Settings are "saved" now, like the firmware saves to NVS
                # before the rebuild.
                ctx.settings.clear()
                ctx.settings.update(next_settings)
                card_speed_applied = None
                if "card_speed" in patch:
                    apply_card_speed(ctx)
                    card_speed_applied = "now" if ctx.sd_state != "none" else "at_mount"

                if needs_reindex:
                    manifest_rel = ctx.player.resolve(ctx.settings["set"])
                    if ctx.fs.read_lines(manifest_rel) is None:
                        ctx.player.rebuild_playlist()  # -> unavailable, empty
                        fs_path = "/sdcard/" + manifest_rel
                        print(
                            f"!!! mock-device: settings saved, but {fs_path} does not "
                            f"exist -- the active manifest/set is now broken until it "
                            f"is created or the config is changed again !!!",
                            file=sys.stderr,
                        )
                        self.send_error_json(500, f"opening {fs_path}: No such file or directory (os error 2)")
                        return
                    index_ms = random.randint(50, 300)
                    time.sleep(index_ms / 1000.0)
                    ctx.player.rebuild_playlist()
                    index = "built"
                else:
                    index = None
                    index_ms = 0
                # tz/tz_name affect local time -> which schedule rule (if
                # any) is in force right now.
                ctx.schedule.evaluate()
                fields = self.playlist_fields(index, index_ms)
                restart_now = False
                if panel_op:
                    if panel_op[0] == "try":
                        ctx.panel.try_(panel_op[1])
                    elif panel_op[0] == "save":
                        ctx.panel.save(panel_op[1])
                    elif panel_op[0] == "confirm":
                        ctx.panel.confirm()
                    else:
                        restart_now = ctx.panel.revert()
                    reboot_required = reboot_required or ctx.panel.restart_needed()
                panel_fields = ctx.panel.fields()
                if restart_now:
                    self.panel_restart()

            extra = {"card_speed_applied": card_speed_applied} if card_speed_applied else {}
            self.send_json(200, {"ok": True, **fields, **extra,
                                 "brightness": ctx.settings["brightness"],
                                 "max_brightness": ctx.settings["max_brightness"],
                                 "reboot_required": reboot_required, **panel_fields})

        def h_reindex(self, path):
            with ctx.lock:
                index_ms = random.randint(50, 300)
                time.sleep(index_ms / 1000.0)
                manifest_rel = ctx.player.manifest_rel()
                if ctx.fs.read_lines(manifest_rel) is not None:
                    ctx.fs.write_bytes(manifest_rel + ".idx", b"MOCKIDX1\n")
                ctx.player.rebuild_playlist()
                fields = self.playlist_fields("built", index_ms)
            self.send_json(200, {"ok": True, **fields})

        def h_overrides_get(self, path):
            _, _, query = self.path.partition("?")
            set_ = query_param(query if "?" in self.path else None, "set")
            if set_ is None:
                set_ = ""
            if any(c in set_ for c in "/\\\0") or ".." in set_:
                self.send_error_json(400, "set must be a single path segment")
                return
            with ctx.lock:
                manifest_rel = ctx.player.resolve(set_)
                meta_map = parse_meta_text_for(ctx.fs, manifest_rel)
            overrides = []
            for gif_path, ov in meta_map.items():
                o = {"path": gif_path}
                if ov.get("never"):
                    o["never"] = True
                if "w" in ov:
                    o["w"] = num_json(ov["w"])
                if "dwell" in ov:
                    o["dwell"] = ov["dwell"]
                elif "dwell_x" in ov:
                    o["dwell_x"] = num_json(ov["dwell_x"])
                if "loops" in ov:
                    o["loops"] = ov["loops"]
                overrides.append(o)
            self.send_json(200, {"set": set_, "playlist": "/" + manifest_rel, "overrides": overrides})

        def h_schedule_get(self, path):
            with ctx.lock:
                doc = ctx.schedule.get_doc()
            self.send_json(200, doc)

        def h_schedule_put(self, path):
            length = int(self.headers.get("Content-Length") or 0)
            if length > SCHEDULE_MAX:
                self.rfile.read(length)
                self.send_error_json(413, f"schedule is {length} B, the limit is {SCHEDULE_MAX} B")
                return
            raw = self.read_body()
            try:
                body = json.loads(raw) if raw.strip() else {}
            except json.JSONDecodeError as e:
                self.send_error_json(400, f"invalid schedule JSON: {e}")
                return
            with ctx.lock:
                err = ctx.schedule.validate_and_set(body)
                if err is not None:
                    code, msg = err
                    if isinstance(msg, dict):
                        self.send_json(code, msg)
                    else:
                        self.send_error_json(code, msg)
                    return
                doc = ctx.schedule.doc
                rules_n = len((doc or {}).get("rules") or [])
                night_present = (doc or {}).get("night") is not None
            self.send_json(200, {"ok": True, "rules": rules_n, "night": night_present})

        def h_wake(self, path):
            raw = self.json_body()
            if raw is None:
                return
            try:
                req = json.loads(raw) if raw else {}
            except json.JSONDecodeError as e:
                self.send_error_json(400, f"invalid JSON: {e}")
                return
            minutes = req.get("minutes") if isinstance(req, dict) else None
            if not isinstance(minutes, int) or isinstance(minutes, bool) or not (0 <= minutes <= WAKE_MAX_MINUTES):
                self.send_error_json(400, f"minutes is at most {WAKE_MAX_MINUTES}")
                return
            with ctx.lock:
                ctx.schedule.wake(minutes)
                wake_until = ctx.schedule.wake_until_str()
            self.send_json(200, {"ok": True, "minutes": minutes, "wake_until": wake_until})

        def h_follow_test(self, path):
            raw = self.json_body()
            if raw is None:
                return
            try:
                req = json.loads(raw) if raw else {}
            except json.JSONDecodeError as e:
                self.send_error_json(400, f"invalid JSON: {e}")
                return
            if not isinstance(req, dict):
                self.send_error_json(400, "invalid JSON: expected a JSON object")
                return
            with ctx.lock:
                saved_host = ctx.settings["follow_host"]
                saved_code = ctx.settings["follow_code"]
            host_in = req.get("host")
            host = host_in.strip() if isinstance(host_in, str) and host_in.strip() else saved_host
            code_in = req.get("code")
            code = code_in if isinstance(code_in, str) else saved_code
            if not host:
                self.send_json(200, {"ok": False, "reachable": False, "error": "no host"})
                return
            h, p = follow_split_host_port(host)
            try:
                vstatus, vdata = follow_http_get(h, p, "/api/v1/get_version", timeout=3.0)
            except Exception:  # noqa: BLE001 - any connect/timeout/protocol failure is "no answer"
                self.send_json(200, {"ok": False, "reachable": False, "error": "no answer"})
                return
            if vstatus != 200:
                self.send_json(200, {"ok": False, "reachable": True, "error": f"get_version: HTTP {vstatus}"})
                return
            try:
                version = str(json.loads(vdata.decode("utf-8")).get("version", ""))
            except Exception:  # noqa: BLE001 - a malformed get_version reply just leaves the version blank
                version = ""
            error = None
            view = system = game = ""
            sstatus = 0
            try:
                sstatus, sdata = follow_http_get(h, p, "/api/v1/get_status", token=code, timeout=3.0)
            except Exception:  # noqa: BLE001 - get_status failing after get_version succeeded is still "reachable"
                sstatus, error = 0, "no answer"
            else:
                if sstatus == 401:
                    error = "401 (wrong code)"
                elif sstatus != 200:
                    error = f"HTTP {sstatus}"
                else:
                    try:
                        body = json.loads(sdata.decode("utf-8"))
                        view = str(body.get("view", ""))
                        system = str(body.get("system", ""))
                        game = clean_game_name(body.get("game_name") or "")
                    except Exception:  # noqa: BLE001 - a malformed reply is "bad reply", not a crash
                        error = "bad reply"
            self.send_json(200, {
                "ok": error is None, "reachable": True, "version": version, "status": sstatus,
                "view": view, "system": system, "game": game, "error": error,
            })

        def h_panel_test(self, path):
            """POST /api/panel/test (opendmd_core::panel_test): start or stop
            the A/B test. The mock has no panel: only the state and the
            replies (a 409 during a trial; ended by a stop, a config PUT or
            PANEL_TEST_S)."""
            raw = self.json_body()
            if raw is None:
                return
            try:
                req = json.loads(raw)
            except json.JSONDecodeError as e:
                self.send_error_json(400, f"invalid JSON: {e}")
                return
            if not isinstance(req, dict) or set(req) - {"setting", "stop"}:
                self.send_error_json(400, "invalid JSON: unknown field, expected `setting` or `stop`")
                return
            st, stop = req.get("setting"), req.get("stop")
            if st is None and stop is True:
                with ctx.lock:
                    ctx.panel.test = None
                self.send_json(200, {"ok": True})
                return
            if not isinstance(st, str) or stop not in (None, False):
                self.send_error_json(400, 'send {"setting":"clk_phase"|"latch_blanking"} or {"stop":true}')
                return
            if st not in ("clk_phase", "latch_blanking"):
                self.send_error_json(400, "setting must be clk_phase or latch_blanking")
                return
            with ctx.lock:
                self.panel_poll()
                why = ("a panel trial is running: keep or undo it first" if ctx.panel.trial
                       else "A panel change is waiting for a restart: restart or undo it first" if ctx.panel.staged
                       else "this driver chip always uses the rising edge: test latch_blanking instead"
                       if st == "clk_phase" and ctx.panel.running["driver"] in ("mbi5124", "dp3246") else None)
                if why:
                    self.send_error_json(409, why)
                    return
                ctx.panel.test = {"setting": st, "until": time.monotonic() + PANEL_TEST_S}
                a, b = ctx.panel.test_variants(st, ctx.panel.running)
            self.send_json(200, {"ok": True, "setting": st, "a": a, "b": b, "left_s": PANEL_TEST_S})

        def h_play(self, path):
            raw = self.json_body()
            if raw is None:
                return
            try:
                req = json.loads(raw)
            except json.JSONDecodeError as e:
                self.send_error_json(400, f"invalid JSON: {e}")
                return
            p = req.get("path") if isinstance(req, dict) else None
            if not isinstance(p, str):
                self.send_error_json(400, "invalid JSON: missing field `path`")
                return
            if req.get("live") is True and not p.startswith("@fx:psi?"):
                self.send_error_json(400, "\"live\" is only for a formula (@fx:psi?src=…)")
                return
            if req.get("over") is True:
                # The editor's formula over the file on screen, until the
                # playlist moves on ("" takes it off).
                if p and not p.startswith("@fx:psi?"):
                    self.send_error_json(400, "\"over\" is only for a formula (@fx:psi?src=…), or \"\" to take it off")
                    return
                if len(p.encode()) > 2048:
                    self.send_error_json(413, f"this formula is too long for the panel ({len(p.encode())} B as sent; the limit is 2048 B)")
                    return
                with ctx.lock:
                    if not p:
                        ctx.player.over = None
                    elif not ctx.player.playlist:
                        self.send_error_json(503, "no playlist is running")
                        return
                    elif req.get("live") is True and not ctx.player.over_on():
                        self.send_error_json(409, "no formula is on the panel now")
                        return
                    else:
                        ctx.player.over = (p, ctx.player.ordinal)
                self.send_json(200, {"ok": True, "path": p, "over": True})
                return
            if p.startswith("@"):
                # A live item: drawn by the player, no file lookup.
                formula = p.startswith("@fx:psi?")
                if formula and len(p.encode()) > 2048:
                    self.send_error_json(413, f"this formula is too long for the panel ({len(p.encode())} B as sent; the limit is 2048 B)")
                    return
                if not is_live(p):
                    self.send_error_json(415, f"{p[:96]}: not a playable file")
                    return
                with ctx.lock:
                    if not ctx.player.playlist:
                        self.send_error_json(503, "no playlist is running")
                        return
                    if formula and (ctx.player.current() or "").startswith("@fx:psi?"):
                        ctx.player.retune(p)
                    elif req.get("live") is True:
                        self.send_error_json(409, "no formula is on the panel now")
                        return
                    else:
                        ctx.player.play_now(p)
                self.send_json(200, {"ok": True, "path": p})
                return
            rel = card_path(p) if p.startswith("/") else None
            if rel is None:
                self.send_error_json(400, "path must be card-absolute and stay on the card")
                return
            with ctx.lock:
                if not ctx.player.playlist:
                    self.send_error_json(503, "no playlist is running")
                    return
                self.dir_scan(rel)
                fpath = ctx.fs.read_path(rel)
                if fpath is None:
                    self.send_error_json(404, enoent(p))
                    return
                with open(fpath, "rb") as f:
                    magic = f.read(4)
                if magic[:4] != b"GIF8":
                    self.send_error_json(415, f"{p}: not a GIF file")
                    return
                ctx.player.play_now(p)
            self.send_json(200, {"ok": True, "path": p})

        def h_stat(self, path):
            _, _, query = self.path.partition("?")
            card = query_param(query if "?" in self.path else None, "path")
            if card is None:
                self.send_error_json(400, "path= required")
                return
            if not card.startswith("/"):
                self.send_error_json(400, "path must be card-absolute")
                return
            rel = card_path(card)
            if rel is None:
                self.send_error_json(400, "path escapes the card")
                return
            obj = {"path": card, "exists": False, "size": None, "is_dir": False}
            if not rel.strip("/"):
                obj.update(exists=True, is_dir=True)
            else:
                rel = rel.rstrip("/")
                self.dir_scan(rel)
                fpath = ctx.fs.read_path(rel)
                if fpath is not None:
                    obj.update(exists=True, size=fpath.stat().st_size)
                elif rel not in ctx.fs.tombstones and ctx.fs.is_dir(rel):
                    obj.update(exists=True, is_dir=True)
            self.send_json(200, obj)

        def h_cache(self, path):
            _, counting, count_ms = ctx.cache.snapshot()
            with ctx.lock:
                total = len(ctx.player.playlist)
                prepared = ctx.auto.prepared()
                auto = ctx.auto.fields()
            self.send_json(200, {
                "total": total,
                "prepared": prepared,
                "state": "counting" if counting else "ready",
                "count_ms": count_ms,
                "auto": auto,
            })

        def h_cache_list(self, path):
            paths = ctx.fs.cache_files()
            self.send_json(200, {"paths": paths, "count": len(paths)})

        def forget_copy(self, gif: str) -> bool:
            copy = cache_path(gif)
            if copy is None:
                return False
            removed = ctx.fs.read_path(copy.lstrip("/")) is not None and ctx.fs.delete(copy.lstrip("/"))
            if removed:
                ctx.cache.invalidate()
                ctx.player.delays.clear()
            return removed

        def h_cache_forget(self, path):
            raw = self.json_body()
            if raw is None:
                return
            try:
                req = json.loads(raw)
            except json.JSONDecodeError as e:
                self.send_error_json(400, f"invalid JSON: {e}")
                return
            p = req.get("path") if isinstance(req, dict) else None
            if not isinstance(p, str):
                self.send_error_json(400, "invalid JSON: missing field `path`")
                return
            if card_path(p) is None or cache_path(p) is None:
                self.send_error_json(400, "path must be a card-absolute /gifs/….gif")
                return
            removed = self.forget_copy(p)
            self.send_json(200, {"ok": True, "path": p, "removed": removed})

        # /cache/.failed: `path<TAB>reason` a line, the GIFs that can never
        # be prepared (the firmware's cache builder writes it; the mock's
        # simulated builder does not).
        def h_cache_failed_get(self, path):
            fp = ctx.fs.read_path(FAILED_REL)
            rows = []
            if fp is not None:
                for line in fp.read_text(encoding="utf-8", errors="replace").split("\n"):
                    gif, _, why = line.partition("\t")
                    if gif.strip():
                        rows.append({"path": gif.strip(), "reason": why.strip()})
            self.send_json(200, rows)

        def h_cache_failed_delete(self, path):
            with ctx.lock:
                cleared = ctx.fs.read_path(FAILED_REL) is not None and ctx.fs.delete(FAILED_REL)
            ctx.cache.invalidate()
            self.send_json(200, {"ok": True, "cleared": bool(cleared)})

        def h_cache_failed_post(self, path):
            raw = self.json_body()
            if raw is None:
                return
            try:
                req = json.loads(raw)
            except json.JSONDecodeError as e:
                self.send_error_json(400, f"invalid JSON: {e}")
                return
            p = req.get("path") if isinstance(req, dict) else None
            why = req.get("reason", "") if isinstance(req, dict) else ""
            if not isinstance(p, str) or not isinstance(why, str):
                self.send_error_json(400, "invalid JSON: missing field `path`")
                return
            if card_path(p) is None or cache_path(p) is None:
                self.send_error_json(400, "path must be a card-absolute /gifs/….gif")
                return
            why = " ".join(why.split())[:200]
            with ctx.lock:
                fp = ctx.fs.read_path(FAILED_REL)
                old = fp.read_bytes() if fp is not None else b""
                known = {l.split(b"\t")[0].strip().lower() for l in old.split(b"\n")}
                added = p.encode().lower() not in known
                if added:
                    if old and not old.endswith(b"\n"):
                        old += b"\n"
                    ctx.fs.write_bytes(FAILED_REL, old + f"{p}\t{why}\n".encode())
            if added and plays_original(why):
                ctx.cache.invalidate()  # counted as done: a recount
            self.send_json(200, {"ok": True, "path": p, "added": added})

        def h_name_check(self, path):
            _, _, query = self.path.partition("?")
            raw = query_param(query if "?" in self.path else None, "name")
            if raw is None:
                self.send_error_json(400, "name= required")
                return
            name = raw.strip().lower()
            valid = bool(HOSTNAME_RE.fullmatch(name))
            if not valid:
                taken, by = None, None
            elif name == ctx.settings["hostname"]:
                taken, by = False, None
            else:
                by = name_check(name)
                taken = by is not None
            self.send_json(200, {"name": name, "valid": valid, "taken": taken, "by": by})

        def h_wifi_scan(self, path):
            time.sleep(1.5)
            self.send_json(200, WIFI_SCAN_RESULTS)

        def h_www_get(self, path):
            self.send_json(200, {**www_info(ctx.www_root, ctx.www_pushed), "staged": ctx.www_staged})

        def h_www_put(self, path):
            length_hdr = self.headers.get("Content-Length")
            length = int(length_hdr) if length_hdr else 0
            if length == 0:
                self.send_error_json(411, "Content-Length required")
                return
            if length > WWW_PARTITION_SIZE:
                # Drain the body so the connection stays usable.
                remaining = length
                while remaining > 0:
                    chunk = self.rfile.read(min(65536, remaining))
                    if not chunk:
                        break
                    remaining -= len(chunk)
                self.send_error_json(413, f"the pack is {length} B, the www partition is {WWW_PARTITION_SIZE} B")
                return
            # `staged` describes this attempt: null if refused at the header.
            ctx.www_staged = None
            data = self.rfile.read(length)
            if data[:4] != b"ODWP":
                self.send_error_json(400, "not a www pack: not a www pack (bad magic)")
                return
            total, vlen = int.from_bytes(data[8:12], "little"), data[18] if len(data) > 18 else 0
            if len(data) < 64 or total != length or vlen > 44:
                self.send_error_json(400, f"not a www pack: the header's length {total} is not the body's {length}")
                return
            # The update, simulated: what is served stays the same, the
            # version reported becomes the pack header's (bytes 20..20+len).
            ctx.www_pushed = data[20:20 + vlen].decode("utf-8", "replace") or None
            ctx.www_staged = ctx.sd_state == "ok"
            self.send_json(200, {**www_info(ctx.www_root, ctx.www_pushed), "staged": ctx.www_staged})

        def h_firmware_get(self, path):
            with ctx.lock:
                card = ctx.sd_state == "ok"  # the firmware reads both off the card
                self.send_json(200, {"version": ctx.version, "build": f"{ctx.version} {ctx.variant}", "variant": ctx.variant,
                                     "flash_size": ctx.flash_size, "pending": None, "last": ctx.fw_last if card else None,
                                     "log": ctx.fw_log if card else None})

        def h_firmware_put(self, path):
            length_hdr = self.headers.get("Content-Length")
            if not length_hdr:
                self.send_error_json(411, "Content-Length required")
                return
            length = int(length_hdr)
            factory = OTA_FACTORY_BYTES[ctx.variant]
            if length > OTA_HEADER_LEN + factory:
                self.drain()
                self.send_error_json(413, f"the bundle is {length} B, at most {OTA_HEADER_LEN + factory} B fits the firmware partition")
                return
            data = self.rfile.read(length)
            t0 = time.monotonic()
            try:
                version = ota_check(data, ctx.ota_pub, ctx.variant)
            except ValueError as e:
                self.send_error_json(422, str(e))
                return
            self.send_json(200, {"ok": True, "version": version, "rebooting": True})
            # The restart: the API is down for RESTART_SECS, then the player
            # "boots" the new firmware (uptime from zero, as after a reboot).
            n = length - OTA_HEADER_LEN
            took = 12.0 + n / 200_000 + (time.monotonic() - t0)
            with ctx.lock:
                old = ctx.version
                ctx.down_until = time.monotonic() + RESTART_SECS
                ctx.version = version
                ctx.fw_last = f"updater +{took:.1f} s: updated to {version} ({n} B, SHA-256 ok, image verified)"
                # The log's end as the firmware reports it (at most 1 KB,
                # whole lines): what the firmware and the updater appended.
                lines = [f"firmware {old}: staged {version} ({n} B), restarting into the updater",
                         "updater +0.4 s: OpenDMD updater started (panel off)",
                         f"updater +{took - 1:.1f} s: written in {took - 2:.1f} s; verifying the image",
                         f"updater +{took:.1f} s: renamed the bundle to /update/done-{version}.bin",
                         ctx.fw_last]
                log = "\n".join((ctx.fw_log or "").splitlines() + lines)
                while len(log.encode()) > 1024 and "\n" in log:
                    log = log.split("\n", 1)[1]
                ctx.fw_log = log
                ctx.restart("firmware_update")
                ctx.player.reset_boot()
                ctx.player.boot_time += RESTART_SECS

        def h_card_format(self, path):
            """http.rs: `{"confirm":true}` (400 otherwise), only a card that
            mounts but cannot be read (409), 202; status `sd.formatting`
            for FORMAT_SECS, then the card reads."""
            body = self.json_body()
            if body is None:
                return
            try:
                req = json.loads(body or b"null")
            except ValueError as e:
                self.send_error_json(400, f"invalid JSON: {e}")
                return
            if not isinstance(req, dict) or req.get("confirm") is not True:
                self.send_error_json(400, 'send {"confirm":true}: everything on the card is erased')
                return
            with ctx.lock:
                sd = sd_fields(ctx)
                why = ("the card is being formatted already" if sd["formatting"]
                       else "no card to format" if not sd["mounted"]
                       else "the card can be read; only an unreadable card is formatted" if sd["readable"]
                       else None)
                if why is None:
                    ctx.format_until = time.monotonic() + FORMAT_SECS
                    ctx.format_error = None
            if why:
                self.send_error_json(409, why)
            else:
                self.send_json(202, {"ok": True, "formatting": True})

        def h_card_full_speed(self, path):
            """http.rs: forget the slow record and run the card at 40 MHz
            again (409 when `card_speed` is safe). ?test_errors=N is
            accepted and ignored: the mock's probe always comes back clean."""
            with ctx.lock:
                if ctx.settings["card_speed"] == "safe":
                    self.send_error_json(409, "card_speed is safe: the card always runs at the safe speed")
                    return
                ctx.sd_slow_record = False
                ctx.sd.update(probe="clean" if ctx.settings["card_speed"] == "fast" else ctx.sd["probe"])
                apply_card_speed(ctx)
                mounted = ctx.sd_state != "none"
            self.send_json(200, {"ok": True, "applied": "now" if mounted else "at_mount",
                                 "clock_khz": 40000 if mounted else None})

        def h_reboot(self, path):
            query = self.path.partition("?")[2] or None
            with ctx.lock:
                ctx.restart("api_reboot")
                ctx.player.reset_boot()
                ctx.panel.boot()
                if query_param(query, "sd_fallback") == "clear":
                    ctx.sd_slow_record = False
                    ctx.sd.update(reboots=0, probe=None)
                    apply_card_speed(ctx)
                io = query_param(query, "sd_io_fail")
                if io and io.isdigit() and int(io) >= 3:
                    # The chain in one step: the failing boot, then the
                    # restart it causes.
                    ctx.sd_slow_record = True
                    ctx.sd.update(reboots=min(ctx.sd["reboots"] + 1, 3))
                    apply_card_speed(ctx)
                    ctx.restart("card_failure")
            self.send_json(200, {"ok": True, "rebooting": True})

        def h_manifest_put(self, path):
            data = self.read_body()
            with ctx.lock:
                obj = self._install_manifest(MANIFEST_REL, data)
            self.send_json(200, obj)

        def h_sets_put(self, path):
            path, _, _ = self.path.partition("?")
            name = percent_decode(path[len("/api/sets/"):])
            if not name or "/" in name or ".." in name:
                self.send_error_json(400, "set name must be a single path segment")
                return
            dest = f"{SETS_DIR}/{name}"
            if not dest.endswith(".txt"):
                dest += ".txt"
            data = self.read_body()
            with ctx.lock:
                obj = self._install_manifest(dest, data)
            self.send_json(200, obj)

        def _install_manifest(self, dest_rel: str, data: bytes) -> dict:
            """Caller holds ctx.lock."""
            idx_rel = dest_rel + ".idx"
            ctx.fs.delete(idx_rel)
            ctx.fs.write_bytes(dest_rel, data)
            active = ctx.player.manifest_rel() == dest_rel
            if active:
                index_ms = random.randint(50, 300)
                time.sleep(index_ms / 1000.0)
                ctx.fs.write_bytes(idx_rel, b"MOCKIDX1\n")
                ctx.player.rebuild_playlist()
                index = "built"
            else:
                index = None
                index_ms = 0
            return {"ok": True, "path": "/sdcard/" + dest_rel, **self.playlist_fields(index, index_ms)}

        def _meta_reply(self, meta_rel: str, manifest_rel: str, include_path: bool) -> dict:
            """A playlist's overrides file was written or removed. Caller
            holds ctx.lock. `index` is "built" only when the edited meta
            belongs to the playlist actually playing (which is always what
            `entries`/`overrides`/`hidden` describe)."""
            active = ctx.player.manifest_rel() == manifest_rel
            if active:
                index_ms = random.randint(30, 150)
                time.sleep(index_ms / 1000.0)
                ctx.player.rebuild_playlist()
                index = "built"
            else:
                index = None
                index_ms = 0
            obj: dict = {"ok": True}
            if include_path:
                obj["path"] = "/" + meta_rel
            obj.update(self.playlist_fields(index, index_ms))
            return obj

        def h_files_put(self, path):
            path, _, _ = self.path.partition("?")
            rel = card_path(percent_decode(path[len("/api/files/"):]))
            if rel is None:
                self.send_error_json(400, "path escapes the card")
                return
            data = self.read_body()
            if is_manifest_path(rel):
                with ctx.lock:
                    obj = self._install_manifest(rel, data)
                self.send_json(200, obj)
                return
            meta_manifest = meta_manifest_for(rel)
            if meta_manifest is not None:
                with ctx.lock:
                    ctx.fs.write_bytes(rel, data)
                    obj = self._meta_reply(rel, meta_manifest, include_path=True)
                self.send_json(200, obj)
                return
            written = ctx.fs.write_bytes(rel, data)
            if rel.lower().startswith(CACHE_DIR + "/"):
                ctx.cache.invalidate()
            elif cache_path("/" + rel) is not None:
                # cache_builder::drop_failed: a GIF written anew drops its
                # cache/.failed line (a failure, or "plays as it is").
                with ctx.lock:
                    fp = ctx.fs.read_path(FAILED_REL)
                    old = fp.read_bytes().split(b"\n") if fp is not None else []
                    keep = [l for l in old if l.split(b"\t")[0].strip().lower() != ("/" + rel).lower().encode()]
                    dropped = len(keep) < len(old)
                    if dropped:
                        ctx.fs.write_bytes(FAILED_REL, b"\n".join(keep))
                if dropped:
                    ctx.cache.invalidate()
            if rel.lower().startswith((CACHE_DIR + "/", "gifs/")):
                # A new copy (a smoothed one has more frames) or a replaced
                # GIF: what the simulated player plays changed.
                with ctx.lock:
                    ctx.player.delays.clear()
            self.send_json(200, {"ok": True, "path": "/" + rel, "bytes": written})

        def h_files_delete(self, path):
            path, _, query = self.path.partition("?")
            rel = card_path(percent_decode(path[len("/api/files/"):]))
            if rel is None:
                self.send_error_json(400, "path escapes the card")
                return
            rel = rel.rstrip("/")
            if query_param(query or None, "recursive") in ("1", "true"):
                top = rel.split("/", 1)[0].lower()
                if top not in (CACHE_DIR, LEGACY_DIR):
                    self.send_error_json(400, "recursive delete is only for cache/ and odm/")
                    return
                if not ctx.fs.is_dir(rel) or rel in ctx.fs.tombstones:
                    self.send_error_json(404, enoent(rel))
                    return
                removed = ctx.fs.delete_tree(rel)
                if top == CACHE_DIR:
                    ctx.cache.invalidate()
                self.send_json(200, {"ok": True, "path": "/" + rel, "removed": removed, "done": True})
                return
            meta_manifest = meta_manifest_for(rel)
            if meta_manifest is not None:
                # A missing overrides file is fine either way.
                with ctx.lock:
                    ctx.fs.delete(rel)
                    obj = self._meta_reply(rel, meta_manifest, include_path=False)
                self.send_json(200, obj)
                return
            with ctx.lock:
                active_rel = ctx.player.manifest_rel()
                active_idx = active_rel + ".idx"
                if rel == active_rel or rel == active_idx:
                    self.send_error_json(409, "that is the active manifest; select another set first")
                    return
                if is_manifest_path(rel):
                    ctx.fs.delete(rel + ".idx")
                # An empty category under gifs/ goes too (http.rs); one with
                # files in it is a 409.
                if rel.lower().startswith("gifs/") and ctx.fs.is_dir(rel):
                    left = ctx.fs.listdir(rel) or []
                    if left:
                        self.send_error_json(409, f"{rel}: not empty")
                        return
                    # --root is never written when there is an overlay: the
                    # overlay's copy goes, the card's is tombstoned.
                    ov = ctx.fs.overlay_dir
                    if ov is not None and (ov / rel).is_dir():
                        shutil.rmtree(ov / rel)
                    if (ctx.fs.root_dir / rel).is_dir():
                        if ov is None:
                            (ctx.fs.root_dir / rel).rmdir()
                        else:
                            ctx.fs.tombstones.add(rel)
                    self.send_json(200, {"ok": True})
                    return
                existed = ctx.fs.delete(rel)
                ctx.player.delays.clear()
                if existed:
                    self.forget_copy("/" + rel)
                    if rel.lower().startswith(CACHE_DIR + "/"):
                        ctx.cache.invalidate()
            if existed:
                self.send_json(200, {"ok": True})
            else:
                self.send_error_json(404, enoent(rel))

        # POST /api/files/move {"from","to"}: http.rs `move_file`. Renames a
        # GIF under gifs/; its prepared copy moves to the new name's bucket
        # (a stale copy already there goes; one that cannot move is
        # removed), and its lines in /sets/.smooth and /cache/.failed follow.
        # Playlists and overrides are the client's.
        def h_files_move(self, path):
            raw = self.json_body()
            if raw is None:
                return
            try:
                req = json.loads(raw)
            except json.JSONDecodeError as e:
                self.send_error_json(400, f"invalid JSON: {e}")
                return
            src = req.get("from") if isinstance(req, dict) else None
            dst = req.get("to") if isinstance(req, dict) else None
            if not isinstance(src, str) or not isinstance(dst, str):
                self.send_error_json(400, "invalid JSON: missing field `from` or `to`")
                return

            def ok(p):
                return (p[:6].lower() == "/gifs/" and not any(c in p for c in "\t\r\n")
                        and card_path(p) is not None and cache_path(p) is not None)

            if not ok(src) or not ok(dst):
                self.send_error_json(400, "from and to must be card-absolute /gifs/….gif paths")
                return
            if src == dst:
                self.send_error_json(400, "from and to are the same")
                return
            srel, drel = src.lstrip("/"), dst.lstrip("/")
            with ctx.lock:
                if ctx.fs.read_path(srel) is None:
                    self.send_error_json(404, src)
                    return
                if srel.lower() != drel.lower() and ctx.fs.read_path(drel) is not None:
                    self.send_error_json(409, f"{dst} already exists")
                    return
                data = ctx.fs.read_path(srel).read_bytes()
                ctx.fs.write_bytes(drel, data)
                if srel != drel:
                    ctx.fs.delete(srel)
                old_c, new_c = cache_path(src).lstrip("/"), cache_path(dst).lstrip("/")
                if old_c.lower() != new_c.lower() and ctx.fs.read_path(new_c) is not None:
                    ctx.fs.delete(new_c)
                copy = "none"
                cp = ctx.fs.read_path(old_c)
                if cp is not None:
                    copy_bytes = cp.read_bytes()
                    ctx.fs.write_bytes(new_c, copy_bytes)
                    if old_c != new_c:
                        ctx.fs.delete(old_c)
                    copy = "moved"
                ctx.player.delays.clear()

                def rename_line(rel):
                    fp = ctx.fs.read_path(rel)
                    if fp is None:
                        return False
                    lines = fp.read_bytes().split(b"\n")
                    hit = False
                    for i, line in enumerate(lines):
                        head, tab, rest = line.partition(b"\t")
                        if head.strip().lower() == src.encode().lower():
                            lines[i] = dst.encode() + tab + rest
                            hit = True
                    if hit:
                        ctx.fs.write_bytes(rel, b"\n".join(lines))
                    return hit

                smooth = rename_line("sets/.smooth")
                failed = rename_line(FAILED_REL)
            ctx.cache.invalidate()
            self.send_json(200, {"ok": True, "from": src, "to": dst, "copy": copy,
                                 "smooth": smooth, "failed": failed})

        def h_files_list(self, path):
            _, _, query = self.path.partition("?")
            query = query if "?" in self.path else None
            dir_param = query_param(query, "dir")
            if dir_param is None:
                dir_param = "/"
            rel = card_path(dir_param)
            if rel is None:
                self.send_error_json(400, "path escapes the card")
                return
            sizes_raw = query_param(query, "sizes")
            sizes = sizes_raw in ("1", "true")
            entries = ctx.fs.listdir(rel)
            if entries is None:
                self.send_error_json(404, enoent(dir_param))
                return
            out = []
            for name, is_dir, size in entries:
                if sizes:
                    sz = 0 if is_dir else (size if size is not None else 0)
                else:
                    sz = None
                out.append({"name": name, "size": sz, "dir": is_dir})
            self.send_json(200, {"dir": dir_param, "entries": out, "count": len(out), "sizes": sizes})

    return Handler


# --------------------------------------------------------------------------
# HTTP handler (proxy mode)
# --------------------------------------------------------------------------

def make_proxy_handler(www_root: Path, target: str, quiet: bool):
    parsed_target = urllib.parse.urlsplit(target)

    class ProxyHandler(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"
        server_version = "OpenDMD-mock-proxy/0.1"

        def log_message(self, fmt, *args):
            pass

        def _run(self, method):
            self._resp_status = None
            self._resp_bytes = 0
            t0 = time.monotonic()
            try:
                self.route(method)
            except (BrokenPipeError, ConnectionResetError):
                pass
            except Exception as e:  # noqa: BLE001
                sys.stderr.write(f"mock-device (proxy): unhandled error on {method} {self.path}: {e!r}\n")
                if self._resp_status is None:
                    self.send_error_json(500, "internal error")
            if not quiet:
                dt_ms = (time.monotonic() - t0) * 1000.0
                print(
                    f"{method:6s} {self.path:60s} {self._resp_status} "
                    f"{self._resp_bytes}B {dt_ms:.1f}ms",
                    file=sys.stderr,
                )

        def do_GET(self):
            self._run("GET")

        def do_POST(self):
            self._run("POST")

        def do_PUT(self):
            self._run("PUT")

        def do_DELETE(self):
            self._run("DELETE")

        def send_json(self, code, obj):
            body = json.dumps(obj, separators=(",", ":")).encode("utf-8")
            self.send_response(code, STATUS_TEXT.get(code, ""))
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            if self.command != "HEAD":
                self.wfile.write(body)
            self._resp_status = code
            self._resp_bytes = len(body)

        def send_error_json(self, code, message):
            self.send_json(code, {"error": message})

        def stream_file(self, actual: Path, logical_rel: str, gzipped: bool):
            size = actual.stat().st_size
            self.send_response(200, "OK")
            self.send_header("Content-Type", mime_for(logical_rel))
            if gzipped:
                self.send_header("Content-Encoding", "gzip")
            self.send_header("Cache-Control", cache_for(logical_rel))
            self.send_header("Content-Length", str(size))
            self.end_headers()
            if self.command != "HEAD":
                with open(actual, "rb") as f:
                    shutil.copyfileobj(f, self.wfile)
            self._resp_status = 200
            self._resp_bytes = size

        def serve_www_rel(self, rel: str) -> bool:
            base = www_root / rel if rel else www_root
            if base.is_dir():
                return False
            gz = base.with_name(base.name + ".gz") if rel else None
            if gz is not None and gz.is_file():
                self.stream_file(gz, rel, gzipped=True)
                return True
            if base.is_file():
                self.stream_file(base, rel, gzipped=False)
                return True
            return False

        def route(self, method):
            path, _, _ = self.path.partition("?")
            if method == "GET":
                if path == "/":
                    if self.serve_www_rel("index.html"):
                        return
                elif path.startswith("/www/"):
                    rel = card_path(percent_decode(path[len("/www/"):]))
                    if rel is None:
                        self.send_error_json(400, "path escapes the card")
                        return
                    if self.serve_www_rel(rel):
                        return
                else:
                    rel_full = percent_decode(path)
                    rel = card_path(rel_full)
                    if rel is None:
                        self.send_error_json(400, "path escapes the card")
                        return
                    if self.serve_www_rel(rel):
                        return
            self.proxy_forward(method)

        def proxy_forward(self, method):
            length = int(self.headers.get("Content-Length") or 0)
            body = self.rfile.read(length) if length else None
            # The page's own Origin (this proxy's host) is dropped: the device
            # refuses a mutation whose Origin is not its own Host. A foreign
            # Origin goes through, to be refused there. Authorization (the
            # API key) is forwarded as it is.
            own = host_port(self.headers.get("Origin") or "") == host_port(self.headers.get("Host") or "-")
            headers = {
                k: v for k, v in self.headers.items()
                if k.lower() not in ("host", "connection") and not (own and k.lower() == "origin")
            }
            conn_cls = http.client.HTTPSConnection if parsed_target.scheme == "https" else http.client.HTTPConnection
            port = parsed_target.port or (443 if parsed_target.scheme == "https" else 80)
            conn = conn_cls(parsed_target.hostname, port, timeout=90)
            try:
                conn.request(method, self.path, body=body, headers=headers)
                resp = conn.getresponse()
                sized = resp.getheader("Content-Length")
                # A sized answer (a card file) goes on as it comes, so a
                # browser that stops reading (a thumbnail's cut) closes the
                # device's connection too, as it would without the proxy.
                data = b"" if sized is not None and self.command != "HEAD" else resp.read()
                self.send_response(resp.status, resp.reason or STATUS_TEXT.get(resp.status, ""))
                for hk in ("Content-Type", "Content-Encoding", "Cache-Control", "WWW-Authenticate", "Retry-After"):
                    v = resp.getheader(hk)
                    if v is not None:
                        self.send_header(hk, v)
                self.send_header("Content-Length", sized if sized is not None else str(len(data)))
                self.end_headers()
                sent = 0
                if self.command != "HEAD":
                    if sized is None:
                        self.wfile.write(data)
                        sent = len(data)
                    else:
                        try:
                            while piece := resp.read(2048):
                                self.wfile.write(piece)
                                self.wfile.flush()
                                sent += len(piece)
                        except (BrokenPipeError, ConnectionResetError):
                            self.close_connection = True
                self._resp_status = resp.status
                self._resp_bytes = sent
            finally:
                conn.close()

    return ProxyHandler


# --------------------------------------------------------------------------
# main
# --------------------------------------------------------------------------

def parse_args():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--root", help="an extracted card copy (gifs/, cache/, filelist.txt, ...)")
    ap.add_argument("--overlay", help="copy-on-write write layer in front of --root")
    ap.add_argument("--www", help="the web app to serve (default: <repo>/opendmd-web/www)")
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=8080)
    ap.add_argument("--dwell", type=float, default=5.0, help="seconds per now-playing item")
    ap.add_argument("--strict", action="store_true",
                    help="behave like the current firmware where the mock differs on purpose: "
                         "a '..' via the catch-all is a 404, stat/play cost a directory scan")
    ap.add_argument("--quiet", action="store_true", help="suppress the one-line-per-request log")
    ap.add_argument("--panel-trial", type=float, default=60.0,
                    help="seconds a panel-timing trial runs before it reverts (the firmware's 60)")
    ap.add_argument("--proxy", help="forward everything but --www to a real device at this URL")
    ap.add_argument("--sd", choices=["ok", "unreadable", "none", "slowed"], default="ok",
                    help="the card: ok, unreadable (mounts, exFAT: nothing plays), none, or slowed "
                         "(readable, set to fast, and already slowed to 20 MHz after errors)")
    ap.add_argument("--taken", default="other-display=192.168.1.42",
                    help="NAME=IP,... : display names other devices answer to (<name>.local); "
                         "GET /api/name-check and PUT /api/config's 409 use them. '' for none")
    ap.add_argument("--name-query", type=float, default=1.2,
                    help="seconds a name check listens when no one answers (the firmware's 1.2)")
    ap.add_argument("--variant", choices=sorted(OTA_MAGICS), default="4mb",
                    help="the flash layout the mock's firmware is built for (GET /api/firmware, /api/status)")
    ap.add_argument("--flash-size", type=int, default=0,
                    help="GET /api/firmware's flash_size, bytes (default: the layout's)")
    ap.add_argument("--ota-pub", help="the Ed25519 public key (32 B) PUT /api/firmware verifies against "
                                      "(default: <repo>/firmware/keys/opendmd.pub)")
    return ap.parse_args()


def main():
    args = parse_args()

    if args.www:
        www_root = Path(args.www).resolve()
    else:
        www_root = (Path(__file__).resolve().parent.parent / "opendmd-web" / "www")
    if not www_root.is_dir():
        print(f"mock-device: warning: --www {www_root} is not a directory", file=sys.stderr)

    stop = threading.Event()

    if args.proxy:
        handler_cls = make_proxy_handler(www_root, args.proxy.rstrip("/"), args.quiet)
        mode_desc = f"proxying everything except --www to {args.proxy}"
    else:
        if not args.root:
            sys.exit("mock-device: need --root <card copy> or --proxy <url>")
        root_dir = Path(args.root).resolve()
        if not root_dir.is_dir():
            sys.exit(f"mock-device: {root_dir}: not a directory")
        overlay_dir = None
        if args.overlay:
            overlay_dir = Path(args.overlay).resolve()
            overlay_dir.mkdir(parents=True, exist_ok=True)
        fs = CardFS(root_dir, overlay_dir)
        ctx = Context(fs, www_root, args.dwell, args.strict)
        ctx.panel.trial_secs = args.panel_trial
        ctx.sd_state = "ok" if args.sd == "slowed" else args.sd
        if args.sd == "slowed":
            ctx.settings["card_speed"] = "fast"
            ctx.sd_slow_record = True
            ctx.sd.update(slowdowns=1, recoveries=1, probe="slowed")
        apply_card_speed(ctx)
        ctx.taken = dict(
            (p.partition("=")[0].strip().lower(), p.partition("=")[2].strip() or "192.168.1.42")
            for p in args.taken.split(",") if p.strip()
        )
        ctx.name_query = args.name_query
        ctx.variant = args.variant
        ctx.flash_size = args.flash_size or (8 << 20 if args.variant == "8mb" else 4 << 20)
        pub = Path(args.ota_pub) if args.ota_pub else Path(__file__).resolve().parent.parent / "firmware" / "keys" / "opendmd.pub"
        try:
            from nacl.signing import VerifyKey
            ctx.ota_pub = VerifyKey(pub.read_bytes())
        except Exception as e:  # noqa: BLE001 - the mock runs on without updates
            print(f"mock-device: no firmware key ({pub}: {e}); every update is refused", file=sys.stderr)
        handler_cls = make_handler(ctx, args.quiet)
        ticker = threading.Thread(target=ticker_loop, args=(ctx, stop), daemon=True)
        ticker.start()
        mode_desc = f"card at {root_dir}" + (f" + overlay {overlay_dir}" if overlay_dir else "")

    server = http.server.ThreadingHTTPServer((args.host, args.port), handler_cls)
    print(
        f"mock-device: http://{args.host}:{args.port}  ({mode_desc}, www={www_root})",
        file=sys.stderr,
    )
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        stop.set()
        server.server_close()


if __name__ == "__main__":
    main()
