# OpenDMD HTTP API

The web app uses only this API, and so can a script. The source of truth
is `firmware/src/http.rs`. `$DMD` in the examples is the player's address
(filled in on the page the player serves).

```sh
DMD=http://opendmd.local
curl -s $DMD/api/status
```

## Ground rules

- **Plain HTTP.** Anyone who can reach the player can change its settings,
  delete files and rewrite its firmware, unless an API key is set (see
  "Security"). Keep it on a trusted network; never forward a port to it.
- **JSON in, JSON out.** Bodies are JSON unless a route takes raw bytes; a
  JSON body needs `Content-Type: application/json` (`415` otherwise:
  `curl --json` sets it).
  Every error is a non-2xx status with `{"error":"<why>"}`, except `405`
  (a route that exists for another method), the server's own `text/html`.
- **Size limits.** JSON bodies ≤ 8 KB (`413` over), the schedule ≤ 2 KB,
  a web app pack ≤ 1,310,720 B. Uploads are streamed, any size, but no data
  for 5 s, or 120 s in all, is a `408`. A body needs a `Content-Length`.
- **One thing at a time.** One server task, at most 7 open connections
  (`HTTP_MAX_SOCKETS`). Files go out and uploads come in one at a time, two
  more waiting; a third gets `503` + `Retry-After: 1` after 2 s. When the
  player needs its memory a listing or transfer is refused up front, `503` +
  `Retry-After: 2`. Retry after the delay, and never in parallel.
- **Slow commands.** A playlist change waits up to 60 s for the index
  rebuild, other commands 10 s; then `504`. No player running: `503`.
- **Paths** are card-absolute (`/gifs/Consoles/SNES_Super_Metroid_01.gif`);
  one that leaves the card is a `400`. Percent-encode them in URLs.

### Fields that recur

Commands that touch the playlist answer with what the player now has:

| field | meaning |
|---|---|
| `entries` | number of entries in the playlist in play |
| `index` | `"loaded"` (the index on the card was current), `"built"` (rebuilt now), `"in-ram"`, or `null` (the playlist was not touched) |
| `index_ms` | how long opening or building the index took, ms |
| `overrides` | per-GIF overrides resolved into the playlist |
| `hidden` | entries hidden from it ("Hide from this playlist") |

A **playlist name** (`set`): `""` is `/filelist.txt`, `filelist.<x>` is
that file at the root, any other name is `/sets/<name>.txt`. No `/` or `..`.

## Playback

### POST /api/play

Play one file now (with the transition, from its prepared copy if there is
one), then the playlist carries on.

Body: `{"path": string}`, a card-absolute GIF path or a live item (see
"Live items").

Reply: `{"ok":true,"path":"…"}`.

Errors: `400` neither card-absolute nor a live item; `404`
the file does not open (what was playing carries on); `415` not a GIF, or a
live item this firmware does not know (`@fx:nonsense`); `503` no playlist is
running; `413` body over 8 KB.

```sh
curl -s $DMD/api/play --json '{"path":"/gifs/Consoles/SNES_Super_Metroid_01.gif"}'
curl -s $DMD/api/play --json '{"path":"@clock?seconds=1&date=1"}'
```

### POST /api/pause

Toggle pause. Reply: `{"ok":true,"paused":bool}`, the new state.

Paused, the panel holds its frame and the player lets go of the file (its
memory goes to preparing copies and downloads), so resuming brings in
the next entry, with the transition from the held frame.

```sh
curl -s -X POST $DMD/api/pause
```

### POST /api/skip

Move on to the next entry now. No body. Reply: `{"ok":true}` (at once; the
skip happens on the player's next frame).

`?back=1` is Previous: play the `recent` entry one older than the one on
screen (again: older still) as `/api/play` does; `404` at the oldest.

```sh
curl -s -X POST $DMD/api/skip
curl -s -X POST "$DMD/api/skip?back=1"
```

## Status and health

### GET /api/status

A snapshot of the player; cheap (the web app polls it twice a second).
Counts are since boot.

| field | meaning |
|---|---|
| `version`, `variant` | firmware version and flash variant (`"4mb"`) |
| `uptime_secs` | seconds since boot |
| `t_ms` | the player's clock, ms, as the reply was built |
| `ip`, `wifi`, `hostname` | address; `"sta"` or `"ap"` (setup mode); name |
| `now_playing` | `{ordinal, path, title, elapsed_ms, frame}`: `elapsed_ms` the position in the file at `t_ms` (loops included); `frame` 1-based |
| `paused` | bool |
| `recent`, `recent_at` | with `?recent=1`: up to 16 files played, newest first, once each; the index of the one on screen |
| `entries`, `manifest` | playlist length; the playlist file in play (`/sdcard/…`) |
| `index_state` | `"ready"`, `"building"` or `"unavailable"` |
| `www_version`, `www` | the web app's version (null if none is good); `GET /api/www`'s object |
| `heap` | `{free, floor, idf_min}`, bytes: now, lowest, the system's lowest |
| `heap_floor_at` | `{task0, task1, largest}`: tasks on each core and the largest free block at the lowest; null until then |
| `refresh_hz` | panel refresh rate |
| `deadline_misses`, `worst_lateness_ms` | all late frames; the latest, ms |
| `misses` | `{steady, switch, threshold_ms, window_ms, worst_steady_ms, by_cause, late_ms}`: `steady`, `switch` frames ≥ `threshold_ms` late in play, or at a GIF change (first frame, transition, first `window_ms`); all late frames by cause (`open`, `read`, `decode`, `transition`, `blit`, `log`, `other`, `wake`) and ms late (`"<5"` … `">=500"`) |
| `gifs_started`, `streams_aborted`, `log_drops` | files opened; responses cut short; log lines dropped |
| `sd` | `{mounted, readable, capacity_bytes, error, attempts, formatting, format_error, clock_khz, fallback, failures, reboots}`: the card; `readable` false if its filesystem cannot be read (exFAT: nothing plays); `formatting`, `format_error` see `POST /api/card/format`; `error` why it is not mounted, not readable or failing; `fallback` slowed after failing at full speed; `failures` errors in the last 60 s; `reboots` card-failure restarts in the last 15 min |
| `reset_reason`, `reset_note` | `poweron`, `software`, `panic`, `task watchdog`, `brownout`, …; `"card failure"`, `"different card"` or null |
| `overrides`, `hidden` | as in "Fields that recur" |
| `mode`, `mode_until` | `normal`, `dim` or `off` (night mode); until `"HH:MM"` |
| `time`, `tz`, `time_synced_s` | `"HH:MM"` (null before network time); zone name; seconds since the last sync |
| `wake_until` | while `/api/wake` holds the schedule off, `"HH:MM"` |
| `schedule` | `{rule, set, until}`: the rule in force (by position), its playlist, its end |
| `cache` | `{state, prepared, total}`, see `GET /api/cache` |
| `context` | Follow RePlayOS: `{source:"replayos", enabled, state, system, platform, console, game, matches, since, step, error}`; `state` `off`, `unreachable`, `attract`, `starting`, `game`, `cooldown`; `since` seconds in it |
| `look`, `glow`, `glow_held` | tint and glow as applied; bytes the glow holds |
| `attract_live` | the live item Follow RePlayOS shows in the menus, else null |
| `max_brightness`, `brightness_safe_max`, `safe_start_until` | the saved ceiling; the safe default (144); uptime second the start-up limit ends (null after) |

```sh
curl -s $DMD/api/status | jq .now_playing
```

### GET /api/stat

`?path=<card path>`. Whether one file or directory exists, and its size.

Reply: `{"path","exists","size","is_dir"}`; `size` in bytes, null for a
directory or a miss. One scan of the parent directory.

Errors: `400` no `path=`, or not card-absolute.

```sh
curl -s "$DMD/api/stat?path=/filelist.txt"
```

### POST /api/reboot

Replies `{"ok":true,"rebooting":true}`, stops any card write, restarts.
Bench test hooks: `?sd_fail=N` (next N card mounts fail),
`?sd_io_fail=N&sd_io_boots=B` (N card reads fail on each of B boots),
`?sd_fallback=clear` (forget the card's slow clock).

```sh
curl -s -X POST $DMD/api/reboot
```

### POST /api/card/format

Formats the card FAT32, erasing it: `{"confirm":true}`, only for a card
that mounts but cannot be read (`sd.readable` false: exFAT, as most cards
over 32 GB come). `202` `{"ok":true,"formatting":true}`; `sd.formatting`
stays true until it is done (~a minute for 64 GB), then `sd.readable` is
true, or `sd.format_error` says why not. `400` no `confirm`; `409` no card,
a readable card, or a format running.

```sh
curl -s $DMD/api/card/format --json '{"confirm":true}'
```

## Settings

### GET /api/config

The saved settings (never the WiFi password or the RePlayOS code).

| field | type, default | meaning |
|---|---|---|
| `wifi_ssid` | string | network to join |
| `wifi_pass_set` | bool | a password is saved (write it as `wifi_pass`) |
| `brightness` | 0..=`max_brightness`, 96 | panel brightness |
| `max_brightness` | 16..=255, 144 | the ceiling `brightness` may not pass |
| `dwell_min`, `dwell_max` | seconds, 3 and 30 | how long a file stays up (at least 1; `dwell_max` ≥ `dwell_min`) |
| `shuffle` | bool, true | shuffle the playlist |
| `platforms` | string, `""` | comma-separated platform prefixes to play, e.g. `"MD,SNES"`; empty = all |
| `set` | string, `""` | the playlist (see "Fields that recur") |
| `hostname` | string, `"opendmd"` | name on the network (`opendmd.local`) |
| `tz`, `tz_name` | strings, `"UTC0"`, `"UTC"` | POSIX time zone (`GMT0IST,M3.5.0/1,M10.5.0`, ≤ 64 characters); its display name (≤ 64) |
| `transition` | `dissolve` | `dissolve`, `wipe_left`, `wipe_right`, `wipe_down`, `wipe_up`, `blinds`, `push`, `fade`, `none` |
| `transition_ms` | 20..=2000, 400 | transition length |
| `auto_prepare` | `idle` | prepare copies by itself: `idle` (when the panel is idle), `always` (also while playing, slower), `off` |
| `follow` | bool, false | Follow RePlayOS |
| `follow_host` | string | the RePlayOS box, `host` or `host:port` (≤ 64) |
| `follow_code_set` | bool | its code is saved (write it as `follow_code`, ≤ 64, one line) |
| `attract_set` | string or null | the playlist shown in RePlayOS's menus; null = the current one |
| `attract` | string, `""` | shown in the menus instead of a playlist: `""`, `"clock"`, `"text:<message>"` (≤ 200 characters, one line), `"fx:<effect>[?options]"` |
| `look` | `colour` | tint: `colour` (`color`), `amber`, `red`, `green`, `blue`, `white` |
| `glow` | `off` | phosphor glow: `off`, `short`, `long` |
| `boot_show` | `short` | start-up show: `off`, `short`, `full` (from the next boot) |

```sh
curl -s $DMD/api/config
```

### PUT /api/config

A partial update: send only the fields to change, by the names above (and
`wifi_pass`, `follow_code` to set those; `""` clears the code). Applied at
once: brightness, look and glow repaint the panel; a transition applies from
the next file; a new `set`, `platforms` or `shuffle` rebuilds the playlist
in place. WiFi and hostname changes are saved but take effect after a
reboot.

Reply: `{"ok":true, entries, index, index_ms, overrides, hidden,
"brightness":N, "max_brightness":N, "reboot_required":bool}` (the playlist
fields as in "Fields that recur"; `reboot_required` is true when the WiFi
settings or the hostname changed).

Errors (nothing is saved): `400` a value out of range or not one of the
names (the message says which), including `brightness N is above the
maximum M; raise Maximum brightness first`; lowering `max_brightness` below
the saved `brightness` lowers it too. `404 {"error":"set not found"}` when
the playlist does not open. `413` body over 8 KB.

```sh
curl -s -X PUT $DMD/api/config --json '{"brightness":64,"transition":"wipe_left","transition_ms":300}'
curl -s -X PUT $DMD/api/config --json '{"set":"consoles","shuffle":true}'
```

### GET /api/wifi/scan

The networks in range (2.4 GHz, named ones, the strongest of each name), a
scan of a couple of seconds: `[{"ssid","rssi","channel","auth","secure"}]`,
`rssi` in dBm. In access-point (setup) mode the list is empty: a scan would
drop the browser that asked. `504` if the scan does not finish in 12 s.

```sh
curl -s $DMD/api/wifi/scan
```

## Files and GIFs

The card's files are served as they are:

| method | path | what |
|---|---|---|
| GET | `/gifs/…` | a GIF |
| GET | `/sets/…` | a playlist (`/sets/<name>.txt`) or its overrides (`.meta`) |
| GET | `/cache/…` | a prepared copy |
| GET | `/filelist.txt`, `/filelist.<x>` | a playlist at the card's root |

With a `Content-Length` and the type from the extension; playlists and
`.meta` are `text/plain` and `no-cache`. A missing file is a `404`.

```sh
curl -s -O $DMD/gifs/Consoles/SNES_Super_Metroid_01.gif
```

### GET /api/files

`?dir=<card path>` (default `/`), `&sizes=1` for sizes. A directory
listing, streamed:

`{"dir":"/gifs","entries":[{"name","size","dir"}],"count":N,"sizes":bool}`

`size` is null without `sizes=1`; with it each costs a directory scan
(minutes for a folder of thousands). `404` no such directory.

```sh
curl -s "$DMD/api/files?dir=/sets&sizes=1"
```

### PUT /api/files/&lt;path&gt;

Write a file anywhere on the card; the body is its bytes. Directories are
created; a file is replaced (the player moves off it first).

Reply: `{"ok":true,"path":"/…","bytes":N}`.

A playlist (`/filelist.*`, `/sets/*`) is installed as by
`PUT /api/sets/<name>`, with its reply; an overrides file (`.meta`) also
rebuilds the playlist if it is in play. With a GIF, also upload its
prepared copy, or forget the old one.

Errors: `400` path escapes the card; `408` the body stalled; `503` busy
(retry).

```sh
curl -s -X PUT --data-binary @Super_Metroid.gif $DMD/api/files/gifs/Consoles/SNES_Super_Metroid_01.gif
```

### DELETE /api/files/&lt;path&gt;

Delete a file (the player moves off it first). A GIF takes its prepared
copy; a playlist its index; an overrides file rebuilds its playlist. An
empty directory under `/gifs/` goes too. Reply: `{"ok":true}`.

`?recursive=1` removes a whole directory, only under `/cache/` (and the
retired `/odm/`), else `400`; 4 s of work a request:
`{"ok":true,"path","removed":N,"done":bool}`, ask again until `done`.

Errors: `404` not there; `409` the playlist in play, or a directory not
empty.

```sh
curl -s -X DELETE $DMD/api/files/gifs/Consoles/SNES_Super_Metroid_01.gif
curl -s -X DELETE "$DMD/api/files/cache?recursive=1"
```

### POST /api/files/move

Rename a GIF: `{"from":"/gifs/A/x.gif","to":"/gifs/B/y.gif"}`. Its prepared
copy follows; playlists and overrides are *not* rewritten (the web app
does that).

Reply: `{"ok":true,"from","to","copy":"moved"|"removed"|"none","smooth":bool,"failed":bool}`.

Errors: `400` not two different `/gifs/….gif` paths; `404` no `from`;
`409` `to` exists (a change of case is fine).

```sh
curl -s $DMD/api/files/move --json '{"from":"/gifs/Other/a.gif","to":"/gifs/Arcade/ARCADE_A_01.gif"}'
```

## Playlists

A playlist is a text file, one card path a line. Read one at
`/sets/<name>.txt`, list them with `GET /api/files?dir=/sets`, pick one with
`PUT /api/config {"set":…}`, delete one with
`DELETE /api/files/sets/<name>.txt`.

### PUT /api/sets/&lt;name&gt;

Write `/sets/<name>.txt` (`.txt` added if missing); the body is the file,
reindexed at once if in play.

Reply: `{"ok":true,"path":"/sdcard/sets/<name>.txt", entries, index,
index_ms, overrides, hidden}`. `400` a name with `/` or `..`.

```sh
printf '/gifs/Consoles/SNES_Super_Metroid_01.gif\n' > consoles.txt
curl -s -X PUT --data-binary @consoles.txt $DMD/api/sets/consoles
```

### PUT /api/manifest

The same for the card's default playlist, `/filelist.txt`.

```sh
curl -s -X PUT --data-binary @filelist.txt $DMD/api/manifest
```

### POST /api/reindex

Reindex the playlist in play (after the card changed some
other way). Reply: `{"ok":true, entries, "index":"built", index_ms, …}`.

```sh
curl -s -X POST $DMD/api/reindex
```

## Overrides and schedule

### GET /api/overrides

`?set=<playlist name>`. A playlist's per-GIF overrides, streamed:

`{"set","playlist":"/filelist.txt","overrides":[{"path", "never"?, "w"?, "dwell"?, "dwell_x"?, "loops"?}]}`

`never` hidden; `w` weight (Favourite 3, Play less 0.5); `dwell` seconds,
or `dwell_x` times the usual; `loops`. To change them, write the `.meta`
beside the playlist (`/sets/<name>.meta`, `/filelist.meta`): a line per GIF,
its path then tab-separated `never`, `w=<n>`, `dwell=<s>` or `dwell=<k>x`,
`loops=<n>`.

```sh
curl -s "$DMD/api/overrides?set="
printf '/gifs/Consoles/SNES_Super_Metroid_01.gif\tw=3\n' > filelist.meta
curl -s -X PUT --data-binary @filelist.meta $DMD/api/files/filelist.meta
```

### GET /api/schedule

The schedule as saved, or `{"rules":[],"night":null}`.

```sh
curl -s $DMD/api/schedule
```

### PUT /api/schedule

Replace the schedule (≤ 2 KB, at most 16 rules):

```json
{"rules":[{"days":[1,2,3,4,5],"from":"18:00","to":"23:00","set":"consoles","brightness":96}],
 "night":{"from":"23:30","to":"07:00","mode":"off","brightness":8}}
```

`days` are 1 (Monday) to 7 (empty/absent: every day); `set` and
`brightness` are optional. Rules are tried in order, first match wins; a
window with `to` ≤ `from` runs past midnight. `night` (every day, beats every
rule) has `mode` `"off"` (panel dark) or `"dim"` (at `brightness`, default
8). A rule never changes the saved settings. Nothing applies before network
time.

Reply: `{"ok":true,"rules":N,"night":bool}`. Errors: `400` a bad time, day,
mode or playlist name (the message names the rule); `404` `set not found:
<names>`; `413` over 2 KB.

```sh
curl -s -X PUT $DMD/api/schedule --json '{"rules":[],"night":{"from":"00:00","to":"07:00","mode":"off"}}'
```

### POST /api/wake

Suspend the schedule for a while (normal mode, the saved playlist):
`{"minutes":N}`, 0..=1440; 0 resumes it now. Reply:
`{"ok":true,"minutes":N,"wake_until":"HH:MM"|null}`. `400` over 1440.

```sh
curl -s $DMD/api/wake --json '{"minutes":60}'
```

## Prepared copies

A prepared copy is a smaller GIF the player plays in place of the original.
The player makes them when idle (`auto_prepare`); the web app makes them
faster and uploads them to `/cache/<Category>/<xx>/<name>.gif`
(`docs/cache-format.md`).

### GET /api/cache

How many of the playlist's files are prepared:

`{"total","prepared","state","count_ms","auto"}`: `total` the playlist's
length; `prepared` how many have a copy (null until first counted);
`state` `"counting"` or `"ready"` (poll until ready); `count_ms` the last count's
time. `auto` is the player's own preparing:
`{"mode","state","reason","policy","session","failed","avg_full_s","avg_drip_s","eta_s","current"}`:
`mode` is `auto_prepare`; `state` `idle`, `running`, `paused` or `done`;
`reason` why it is not running (`off`, `playing`, `gif on screen`, `low
memory`, `busy`, `transition`, `reindex`, `no playlist`, `card error`);
`policy` `full` or `drip` (the pace); `session` copies made since boot;
`failed` GIFs it cannot prepare; `avg_*_s` seconds per file at each pace;
`eta_s` seconds left at full pace; `current` the GIF being prepared.

```sh
curl -s $DMD/api/cache
```

### GET /api/cache/list

Each prepared copy, streamed: `{"paths":["/cache/…"],"count":N}`.
Subtract it from a playlist: what is left to prepare.

```sh
curl -s $DMD/api/cache/list | jq .count
```

### GET /api/cache/failed

The GIFs that cannot be prepared (nor played), streamed:
`[{"path":"/gifs/…","reason":"more than 256 colours"}]`, `[]` if none.

```sh
curl -s $DMD/api/cache/failed
```

### POST /api/cache/failed

Record a GIF the browser could not prepare either, so the player skips it
too: `{"path":"/gifs/….gif","reason":string}` (the reason: one line,
≤ 200 characters). Reply: `{"ok":true,"path","added":bool}` (`added`
false if it was there). `400` not a `/gifs/….gif` path.

```sh
curl -s $DMD/api/cache/failed --json '{"path":"/gifs/Other/broken.gif","reason":"image truncated"}'
```

### DELETE /api/cache/failed

Forget the whole list, so replaced GIFs are tried again. Reply:
`{"ok":true,"cleared":bool}` (`cleared` false if there was no list).

```sh
curl -s -X DELETE $DMD/api/cache/failed
```

### POST /api/cache/forget

Delete one GIF's prepared copy (after replacing the GIF):
`{"path":"/gifs/….gif"}`. If it is on the panel the player moves on first.
Reply: `{"ok":true,"path","removed":bool}`. `400` not a `/gifs/….gif` path.

```sh
curl -s $DMD/api/cache/forget --json '{"path":"/gifs/Consoles/SNES_Super_Metroid_01.gif"}'
```

## Follow RePlayOS

With `follow` on, the panel follows a RePlayOS box: the game's art while a
game runs, the attract playlist (or `attract`) in the menus. Configure it
with `PUT /api/config` (`follow`, `follow_host`, `follow_code`,
`attract_set`, `attract`); its state is `context` in `/api/status`.

### POST /api/follow/test

Ask the box for its version and status, without saving anything:
`{"host"?: string, "code"?: string}` (the saved ones where absent; an empty
body is fine). Takes up to ~20 s.

Reply: `{"ok","reachable","version","status","view","system","game","error"}`;
`status` is the box's HTTP status (401: wrong code). An early failure has
only `ok`, `reachable`, `error` (and `version`).

```sh
curl -s $DMD/api/follow/test --json '{"host":"192.168.1.20","code":"1234"}'
```

## Network

The player joins the WiFi network in its settings. With none, or after
failing to join twice, it raises an open network, `OpenDMD-Setup`, as a
captive portal (unknown paths redirect to `/`),
and restarts every 10 minutes to try again.

| method | path | what |
|---|---|---|
| GET | `/` | the web app (in setup mode with no good web app in flash, the setup page) |
| GET | `/setup` | the built-in setup page: join a network |
| GET | `/hotspot-detect.html`, `/generate_204`, … | captive-portal probes: a redirect to `/` in setup mode, else 404 |

Set the network with `PUT /api/config`, then reboot:

```sh
curl -s -X PUT $DMD/api/config --json '{"wifi_ssid":"MyNet","wifi_pass":"secret"}'
curl -s -X POST $DMD/api/reboot
```

## Security

Plain HTTP: this keeps out casual use on a trusted network (a guest, a
script, another web page in your browser), not someone on the same WiFi.

- A `POST`/`PUT`/`DELETE` whose `Origin` is another host than its `Host`
  is a `403` (no `Origin`, as from curl, is fine).
- **API key** (Settings → Advanced → Security): with `api_key` set (`PUT
  /api/config`, 8..64 of `A-Z a-z 0-9 . _ ~ -`, `""` = off; `GET` shows
  `api_key_set`), the API and the card's files need `Authorization: Bearer
  <key>` or `?key=<key>`, else `401`. Not the web app's files or `/setup`,
  nor in setup mode (`OpenDMD-Setup`), where a lost key can be cleared.

```sh
K='Authorization: Bearer <key>'
curl -s -H "$K" $DMD/api/status
curl -s -X PUT $DMD/api/config -H "$K" --json '{"api_key":""}'
```

## Web app update

The web app lives in the player's flash as one file, a www pack
(`docs/www-pack.md`), not on the card. `make web` builds it as
`opendmd-web/www.pack`; a release has it as `dist/opendmd-<version>-www.pack`.
Its files are served at `/…` and `/www/…`.

### GET /api/www

`{"version","bytes","entries","ok","error","staged"}`: the pack's version
(`git describe` of its build), size, file count, whether it verified, why
not (`error`: erased, damaged, an update cut short, being written), and
whether the last update attempt since boot was staged on the card (null:
none, or the last one was refused at its header).

```sh
curl -s $DMD/api/www
```

### PUT /api/www

Write a new pack (the whole file). Its header is checked before anything
is erased: not a pack, an unknown format, or a length that is not the
body's is a `400` (sent at once, before the rest of the body), and the app
in flash stays. With a card in, it is staged on the card and read back
whole first (size and CRC; a read the card fails is retried), so a `408`
(cut short), `422` (bad CRC) or `500` (the card would not read it back)
leaves the old app in place. Then it is written to flash from the card
(~7 s for 800 KB in all) and served at once; no reboot. If that write
still fails (a `500`), pages are `503` and the checked pack stays on the
card: the next restart writes it, or push again. Without a card (or one
that cannot be written), after a `408`/`422` pages are `503` until a good
pack is written.

Reply: `GET /api/www`'s object; `staged` false means no card staging. `411` no length; `413` too big. Any good
pack is accepted, older or newer: check `GET /api/www` reports the version
you sent (`make www-push HOST=…` does).

```sh
curl -s -X PUT --data-binary @opendmd-web/www.pack $DMD/api/www
curl -s $DMD/api/www
```

## Firmware update

A signed bundle (`dist/opendmd-<version>.odfw`, `make fw-release`) is
staged on the card, checked and installed (`docs/ota.md`). Needs a card.
`make fw-ota HOST=…` does it.

### GET /api/firmware

`{"version","build","variant","pending","last","log"}`: the running
firmware's version (the one a bundle for it carries, also `version` in
`/api/status`; `build`: its image's descriptor, `"<version> 4mb"`), a
staged update not yet installed (or null), the last line of the card's
`/update/log.txt` and its end (`log`: at most 1 KB of whole lines, what
the firmware and the updater wrote; both null without a card or a log).

```sh
curl -s $DMD/api/firmware
```

### PUT /api/firmware

The bundle as the body (`411` no length, `413` too big). Its header, size,
SHA-256 and Ed25519 signature are checked: a failure is a `422` with the
reason and nothing is staged; cut short `408`; no card `503`. The
signature is checked when the 140-byte header is in, so a bundle signed
with another key is refused then, without waiting for the rest of the
body (curl stops sending; the device reads off what is in flight for up
to 2 s before it closes). Any validly signed bundle is accepted, older
versions too. Then `{"ok":true,"version":"…","rebooting":true}`, and the
panel is dark for ~45 s (from the end of the upload to playback) while it
installs.

```sh
curl -s -X PUT --data-binary @dist/opendmd-0.4.0.odfw $DMD/api/firmware
```

## Live items

Paths beginning with `@` are drawn, not read from the card. Use them with
`POST /api/play`, in a playlist line, and in `attract`. Options follow `?`
as `key=value&…`; an unknown key or bad value is ignored (the default
stays). Colours are `RRGGBB` (`%23RRGGBB` or `#RRGGBB` also
work).

**`@clock`** — the time (local, from the network; `--:--` until set).
`style=big|small` (default big), `seconds=1`, `date=1` (the date under it),
`h12=1` (12-hour), `color=RRGGBB`. One loop is one second.

```sh
curl -s $DMD/api/play --json '{"path":"@clock?style=small&seconds=1&color=ff8000"}'
```

**`@text`** — scrolling text. `msg=<url-encoded>` (≤ 200 characters, ASCII
shown), `speed=<ms per pixel>` (20..=1000, default 40),
`font=big|normal|small` (default big), `color=RRGGBB`, `scroll=0` (still
and centred if it fits). One loop is one pass across the panel.

```sh
curl -s $DMD/api/play --json '{"path":"@text?msg=Game%20over&speed=30&color=00ff00"}'
```

**`@fx:<effect>`** — a live effect: `tunnel`, `rotozoom`, `starfield` or
`wire`. For all: `speed=1..4` (default 2), `fps=100|50|25` (default 100),
`palette=fire|ice|chrono|amber|rainbow`, `cycle=1` (cycle the palette).
`tunnel`: `twist=0..3`, `drift=1`, `texture=checker|rings|noise`,
`layers=1|2`. `rotozoom`: `zoom=1..4`, `floor=1` (a perspective floor),
`wobble=0..3`, `texture=…`, `layers=1|2`. `starfield`: `density=1..3`,
`warp=0`. `wire`: `shape=cube|octahedron|icosahedron|torus|tesseract|morph`,
`color=RRGGBB`, `shade=0`. An unknown effect is a `415`.

```sh
curl -s $DMD/api/play --json '{"path":"@fx:wire?shape=tesseract&speed=3"}'
```

**`@boot`** — the start-up show. `len=short|full|card` (`card`: the status
card), and for the card `v=<version>`, `ip=<address>`, `ap=<network>`,
`note=<text>` (≤ 32 characters each).

```sh
curl -s $DMD/api/play --json '{"path":"@boot?len=full"}'
```

Also `@power-on` (the power-on effect) and `@card:<title>|<subtitle>` (a
title card, 3 s).
