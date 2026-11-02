# OpenDMD HTTP API

Everything the web app does goes through this API, and a script can do the
same. The source of truth is `firmware/src/http.rs`. In the examples `$DMD`
is the player's address.

```sh
DMD=http://opendmd.local
curl -s $DMD/api/status
```

## Ground rules

- **Plain HTTP.** Anyone who can reach the player can change its settings,
  delete files and rewrite its firmware, unless an API key is set (see
  "Security"). Keep it on a trusted network; never forward a port to it.
- **JSON in, JSON out.** Bodies are JSON unless a route takes raw bytes,
  with `Content-Type: application/json` (`415` otherwise; `curl --json`
  sets it). Every error is a non-2xx status with `{"error":"<why>"}`, except
  `405` (a route that exists for another method), the server's `text/html`.
- **Size limits.** JSON bodies ≤ 8 KB (`413` over), the schedule ≤ 2 KB,
  a web app pack ≤ 1,310,720 B. Uploads are streamed, any size, but no data
  for 5 s, or 120 s in all, is a `408`. A body needs a `Content-Length`.
- **One thing at a time.** One server task, at most 7 open connections
  (`HTTP_MAX_SOCKETS`). Transfers and the listings (`/api/files`,
  `/api/cache/list`, `/api/cache/failed`) run one at a time, two more
  waiting; a third transfer gets `503` + `Retry-After: 1` after 2 s, a
  listing at once. Short of memory, a listing
  or transfer is refused up front: `503` + `Retry-After: 2`. Retry after the
  delay; do not fire requests in parallel.
- **Slow commands.** A playlist change waits up to 60 s for the index
  rebuild, other commands 10 s, then `504`; no player running, `503`.
- **Paths** are card-absolute (`/gifs/Consoles/SNES_Super_Metroid_01.gif`),
  percent-encoded in URLs; one that leaves the card is a `400`.

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

Body: `{"path": string}`, a card-absolute GIF path, a Formula (a bundled
one by name, `@formula:<stem>`; one of yours on the card,
`/formulas/<stem>.psi`, maybe with `?a=…`; or inline, `@formula?psb=…`) or
a live item (see "Live items"). `"live":true`: only replace a formula on screen (`409` if
none is; `400` for any other path). `"over":true` with a formula: draw it
over the file on screen instead of playing it, until the playlist moves on
to another file (in place of the `look_formula` meanwhile); with `"live"`
it changes that formula (`409` once the playlist has moved on); `"path":""`
takes it off. `"over"` skips the card-path check (the path is a formula or
`""`); with no playlist running it is a `503`; a formula that covers the GIF
entirely (it reads none of `ur ug ub ul` and nothing shows through it) or is
too heavy to draw over GIFs is a `400` with the reason.

A Formula played this way (not `"over"`) stays as long as `formula_keep`
says: one turn of the dwell (the default), a minute, five, or until
something else plays (another `/api/play`, `/api/skip` or Previous, a
playlist change). A live edit keeps it kept. While it is kept, `/api/status`
`now_playing.kept_s` says for how long (`0`: until changed).

Reply: `{"ok":true,"path":"…"}` (with `"over":true` when drawn over).

Errors: `400` neither card-absolute nor a live item, or a bad formula;
`404` the file does not open, or no bundled Formula has that name
(`there is no bundled Formula by that name`; what was playing carries on); `415` not a
GIF, a live item this firmware does not know (`@nonsense`), or a retired
`@fx:` path: the built-in effects are gone (`the built-in effects are gone:
Formulas replace them (play one as @formula?psb=…)`); `503` no
playlist running; `413` body over 8 KB, or an inline formula over 6,144 B.

```sh
curl -s $DMD/api/play --json '{"path":"/gifs/Consoles/SNES_Super_Metroid_01.gif"}'
curl -s $DMD/api/play --json '{"path":"@clock?seconds=1&date=1"}'
```

### POST /api/pause

Toggle pause. Reply: `{"ok":true,"paused":bool}`, the new state.

While paused the panel holds its frame and the player lets go of the file
(its memory goes to preparing copies and downloads), so resuming brings in
the next entry, with the transition, from the held frame.

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

A cheap snapshot (the web app polls it twice a second); counts since boot.

| field | meaning |
|---|---|
| `version`, `variant` | firmware version and flash layout (`"4mb"`, the DevKit; `"8mb"`, an 8 MB board in development) |
| `psi_abi` | the compiled Formulas this player plays (`@formula?psb=…`, a card's `.psb`); `null` in a build without Formulas |
| `uptime_secs` | seconds since boot |
| `t_ms` | the player's clock, ms, as the reply was built |
| `ip`, `wifi`, `hostname` | address; `"sta"` or `"ap"` (setup mode); name |
| `mdns` | the name mDNS answers to (`<mdns>.local`): `hostname`, or `<hostname>-2` when another device has it; `""` without mDNS |
| `now_playing` | `{ordinal, path, title, elapsed_ms, frame}`: `elapsed_ms` the position in the file at `t_ms` (loops included); `frame` 1-based; `kept_s` while a Formula sent with `/api/play` is kept on the panel (`formula_keep`): its seconds from the start (`0`: until something else plays) |
| `paused` | bool |
| `recent`, `recent_at` | with `?recent=1`: up to 16 files played, newest first, once each; the index of the one on screen |
| `entries`, `manifest` | playlist length; the playlist file in play (`/sdcard/…`) |
| `index_state` | `"ready"`, `"building"` or `"unavailable"` |
| `www_version`, `www` | the web app's version (null if none is good); `GET /api/www`'s object |
| `heap` | `{free, floor, idf_min, transitions_cut}`: bytes now, lowest, the system's lowest; transitions the heap gate cut since boot (a probe build adds `main_stack`, the main task's stack bytes never used, and `pushed`, pixels pushed to the panel) |
| `heap_floor_at` | `{task0, task1, largest}`: tasks on each core and the largest free block at the lowest; null until then |
| `refresh_hz` | panel refresh rate |
| `deadline_misses`, `worst_lateness_ms` | all late frames; the latest, ms |
| `misses` | `{steady, switch, threshold_ms, window_ms, worst_steady_ms, by_cause, late_ms}`: frames ≥ `threshold_ms` late in play (`steady`) or at a GIF change (`switch`: first frame, transition, first `window_ms`); all late frames by cause (`open`, `read`, `decode`, `transition`, `blit`, `log`, `other`, `wake`) and ms late (`"<5"` … `">=500"`); `recent` the last 8 late frames (newest last), each `{late_ms, cause, open, read, decode, blit, log, other, wake, transitioning, advance_us, step_us, at_s}`: its cause (as above), each step's µs, whether a transition was running, the engine tick split into the GIF/transition advance (`advance_us`, with the incoming GIF's opens and decodes) and the composite step (`step_us`), and seconds since boot (a diagnostic) |
| `gifs_started`, `streams_aborted`, `log_drops`, `stream_stack` | files opened; responses cut short; log lines dropped; the file task's least unused stack after a job, bytes (null before one) |
| `sd` | `{mounted, readable, capacity_bytes, card_id, error, attempts, formatting, format_error, clock_khz, fallback, failures, reboots, probe, slowdowns, recoveries, fat_index}`: the card; `readable` false if it mounts but its filesystem cannot be read (exFAT: nothing plays); `card_id` which card this is (16 hex digits from its serial number, its volume's serial and label, and its size: another card, or the same card formatted again, gives another; null with no readable card), for keeping a browser's per-card data apart; `formatting`, `format_error`: `POST /api/card/format`; `error` why it is unmounted, unreadable or failing; `clock_khz` its SPI clock (20000 safe, 40000 fast; null unmounted); `fallback` a `fast` card slowed to 20 MHz after errors; `failures` errors in 60 s; `reboots` card-failure restarts in 15 min; `probe` the check of a card at full speed soon after it mounts: null, `waiting`, `running`, `clean`, `slowed`, `skipped`; `slowdowns` times `fast` slowed the card in place this boot; `recoveries` failed transfers the card was stopped after (CMD12); `fat_index` `{files, hits, misses, builds, stale, raw_open, raw_reads, raw_fallbacks, slots_full, intent_refused, delta}` diagnostics of the index that opens GIFs and their prepared copies without a directory scan (`delta`: files written, renamed or removed since it was built, which it already knows) and of the player's card opens and reads (both past FatFs's volume lock; opens refused while a write holds the file); `write_yields`, `write_yield_us` the card writes that stepped aside between their chunks for reads, and how long (at most 5 ms a chunk); `settings_store` where the settings are kept: `card` (`/opendmd.json`, see Settings), `nvs (no card)`, or `nvs (card write failed: …)` (technical: an app showing it says "Settings are kept on the card") |
| `reset_reason`, `reset_note` | `poweron`, `software`, `panic`, `interrupt watchdog`, `task watchdog`, `brownout`, `external`, …; why this boot's restart was made in plain words — `"card failure"`, `"different card"`, `"restarted on request"`, `"restarted to apply a card speed change"`, `"crashed (panic)"`, `"restarted without a recorded reason"`, … — or null after a power-on |
| `restarts` | the last 8 restarts, newest first (this boot's first), kept across restarts, cleared by a power-on: `{reason, reset, uptime_s, heap_free, largest, where?}`. `reason`: `api_reboot`, `firmware_update`, `ap_timeout` (setup network timed out), `card_full_speed`, `card_speed`, `fresh_heap` (playlist index needed a fresh heap), `panel_trial`, `card_failure`, `new_card`, `card_lost` (SPI device lost), `card_retry`, `panic`, `stack_overflow`, `diag_abort` (a `diag-heap` build's heap check), or, when the restart wrote nothing, from the reset alone: `power_on` (also the EN/reset button: the ESP32 reports it as a power-on, and the list starts again), `brownout`, `unrecorded` (a software restart), `unrecorded_crash`. `reset`: the chip's reset after it (`reset_reason`'s words; null if no boot has seen it yet). `uptime_s`, `heap_free`, `largest`: when it was written (for `unrecorded`/`unrecorded_crash`/`brownout`, the last heartbeat's, ≤ 30 s earlier; null when unknown; `largest` null for `panic`, `stack_overflow` and `diag_abort`, which do not walk the heap). `where`: a panic's `file:line message` (message ≤ 48 B), the overflowing task (`task <name>`), the heap check's site, or for `unrecorded_crash` the last failed allocation if it came at most 35 s before the last heartbeat or after it (up to ~65 s before the crash; it may have been a deliberate, handled probe) |
| `overrides`, `hidden` | as in "Fields that recur" |
| `mode`, `mode_until` | `normal`, `dim` or `off` (night mode); until `"HH:MM"` |
| `time`, `tz`, `time_synced_s` | `"HH:MM"` (null before network time); zone name; seconds since the last sync |
| `wake_until` | while `/api/wake` holds the schedule off, `"HH:MM"` |
| `schedule` | `{rule, set, until}`: the rule in force (by position), its playlist, its end |
| `cache` | `{state, prepared, total}`, see `GET /api/cache` |
| `context` | Follow RePlayOS: `{source:"replayos", enabled, state, system, platform, console, game, matches, since, step, error}`; `state` `off`, `unreachable`, `attract`, `starting`, `game`, `cooldown`, for `since` s |
| `look`, `glow`, `glow_held` | tint and glow as applied; bytes the glow holds |
| `formula_look` | null, or the formula drawn over what plays: `{"source": "setting"\|"over_gif", "held": bytes (0: no room, the picture goes up as it is), "frame_us": mean render, "frame_ms": its own frame time (0: it follows the picture), "jit": as `now_playing.formula.jit`, for the look's kernel}`, with `"error"` when the saved `look_formula` can no longer be drawn (e.g. after a firmware change) |
| `attract_live` | the live item Follow RePlayOS shows in the menus, else null |
| `max_brightness`, `brightness_safe_max`, `safe_start_until` | the saved ceiling; the safe default (144); uptime second the start-up limit ends (null after) |
| `scan` | null when the firmware cannot read the panel's scan position; else `{row_us, sync, blits, torn, in_pair, waited, wait_us, wait_max_us, gave_up, split_on, split, split_ok, unsplit, spin_us, retimed, beam}`, counts since boot: `row_us` one row pair's share of the measured refresh (µs), `sync` = `panel_sync`; of the `blits`, `torn` left a seam (a row pair shown complete a refresh after another) and `in_pair` wrote a pair while it was being scanned; with `panel_sync`, `waited` waited for the scan (`wait_us` in all, wrapping; `wait_max_us`) and `gave_up` could not within the frame's time; with `panel_split` (`split_on`), `split` blits went up in two passes, `split_ok` of them with the bottom exactly one refresh after the top, and `unsplit` went up whole for want of time; `spin_us` the waits' busy part (µs in all, wrapping; the rest is asleep); `retimed` Formula frames whose tick came half a frame or more late, so the Formula's schedule restarted from the tick; `beam` racing the beam, the way Formulas play (a Formula's frames drawn in the panel's row-pair order, each pair written as soon as it is drawn, behind the scan, on the panel's refresh: every refresh for a 100 fps Formula, every 2nd for 50 fps; the governor holds a heavy one to every 2nd, 4th …; not a Formula's first frame, its JIT check, under the glow or a formula look, or while a transition runs: those go up as GIFs do): `{frames, late_frames, late_pairs, late_starts, retargets, waits, pair_waits, wait_us, write_us, pair_us, first_us, last_us, unread}` — raced Formula frames, those with a pair written past its window (`late_pairs` in all: each shows a refresh late, a seam for that refresh), the starts past their window's late edge and of them those moved to the next refresh instead (a Formula held for two refreshes or more: over 1 late frame in 1,024 holds one whose own frame time is over three quarters of its budget for 2 for ~4,096 frames; a lighter one keeps its rate), frames that waited for their window at the start and pairs that waited for theirs (`wait_us` in all, wrapping), the µs spent writing pairs and in the pair hook altogether, writes included (both wrapping), the running means of when a frame's first and last pair are drawn (µs after its start), and frames not raced because the scan could not be read |
| `display` | how the display loop kept its time, since boot: `{wakes, wake_late, wake_max_us, spin_us, publish_skipped, status_lock_us, status_build_us, status_cached, pre_start, commands}` — the loop's wakes, those over 100 µs after their time and the latest (µs), the µs it spun waiting for its frame (it blocks until 0.4 ms before, then spins), frames whose status publish was skipped because a status read held it (the display never waits for one), the last status read's time holding that lock (a copy) and building in all (µs), status reads answered from the last answer (it is reused for 100 ms); `pre_start {n, us, mean_us, max_us, spikes}` a raced frame's time from the wake to its start (the loop's work, the tick, the frame's steps and draw stage; `spikes` over its running mean by 300 µs); `commands {n, us, max_us}` the loop's handling of what the HTTP handlers asked (loops that had any, µs in all, the longest); `buffers` where the display's hot memory is (a measurement: each `{at, bytes, pool}` or `null`, `pool` IDF's SRAM pool such as `SRAM2/15` or `SRAM1/7`): `formula` (the playing Formula's program and VM scratch), `ink`, `plane`, `kernel` (its JIT code), `arena` (`formula_arena`), `canvas`, `dma_rows` (the panel's DMA rows, `{at, end, bytes, pool, pool_end}`), `dma_desc`; `arena` `{hits, busy}` while `formula_arena` is in force, else `null`; `card_read_max_us` the player's longest single card read since the last `?take_read_max=1`; `uploads` `{paced, rate_kb_s, paced_ms, on_pct}`: uploads are read at `rate_kb_s` at most while a Formula plays near its frame budget (its mean render over `on_pct` % of the frame time, 60 by default, until 10 points under), full speed otherwise; `paced_ms` the time slept for it since boot. A dev field of `PUT /api/config`, `pace_on_pct` (1..100, live only, never saved), moves the start for a test |
| `?pools=1` | answers `{"pools":{"SRAM2/15":{used, blocks, free}, …}}` instead of the status: one walk of the heap, bytes in use and free per SRAM pool (a measurement) |
| `?take_read_max=1` | answers `{read_max_us, open_max_us}` instead of the status: the player's longest card read and open since the last ask, then both start again from 0 |

```sh
curl -s $DMD/api/status | jq .now_playing
```

### GET /api/stat

`?path=<card path>`: whether a file or directory exists, its size.

Reply: `{"path","exists","size","is_dir"}`; `size` in bytes, null for a
directory or a miss.

Errors: `400` no `path=`, or not card-absolute.

```sh
curl -s "$DMD/api/stat?path=/filelist.txt"
```

### POST /api/reboot

Replies `{"ok":true,"rebooting":true}`, stops any card write, restarts.
Bench test hooks: `?sd_fail=N` (next N card mounts fail),
`?sd_io_fail=N&sd_io_boots=B` (N card reads fail on each of B boots),
`?sd_fallback=clear` (forget the card's slow clock at the next boot; see
`POST /api/card/full-speed` for the live way), and in `bench` builds only
`?panic=1` (replies `{"ok":true,"panicking":true}`, then panics on purpose;
`reset_reason` in `/api/status` reads `panic` after the restart, and
`restarts[0]` is `{"reason":"panic", "where":"http.rs:… test panic …"}`), and
`?overflow=1` (replies `{"ok":true,"overflowing":true}`, then overwrites the
httpd task's stack canary so FreeRTOS's overflow check fires; `restarts[0]`
is `{"reason":"stack_overflow", "where":"task httpd"}`).

```sh
curl -s -X POST $DMD/api/reboot
```

### POST /api/card/format

Formats the card FAT32, erasing it: `{"confirm":true}`, only for a card
that mounts but cannot be read (`sd.readable` false: exFAT). `202` `{"ok":true,"formatting":true}`; `sd.formatting`
until done (~a minute for 64 GB), then `sd.readable` true or
`sd.format_error`. `400` no `confirm`; `409` no card, a readable one, or a
format running.

```sh
curl -s $DMD/api/card/format --json '{"confirm":true}'
```

### POST /api/card/full-speed

"Try full speed again": forgets this card's slow clock (`sd.fallback`) and
returns it to 40 MHz, in place (no restart) when a card is in; with
`card_speed` `fast` it is then checked again, and slowed again at the first
error. `{"ok":true,"applied":"now","clock_khz":40000}`; `"applied":
"at_mount"` with no card; `"restart"` (with `"rebooting":true`) only if the
clock could not be changed in place. `409` when `card_speed` is `safe`. Test
hook (bench builds; a release ignores it): `?test_errors=N` fails the next N card transfers at full speed (the
check is skipped), to watch `fast` slow a good card down.

```sh
curl -s -X POST $DMD/api/card/full-speed
```

## Settings

**Where they are kept.** The settings that belong to what plays travel
with the card, in `/opendmd.json` at its root: brightness and its
ceiling, dwell, shuffle, platforms, the playlist (`set`), the time zone,
the schedule, transition, `auto_prepare`, Follow RePlayOS on/off and its
host, attract, look, glow, `look_formula`, `boot_show`, `panel_sync`,
`panel_split`, `formula_jit`, `formula_arena`, `formula_keep`. A change is
on the card the moment it is saved, whatever plays. The device keeps the
rest itself, never on the card: the WiFi details and name, the API key
and the RePlayOS code (secrets), the card's clock and its slow-clock
record, and the panel timing. It also keeps a copy of everything, what it
runs on until a card is read and with no card at all.

The file is JSON, by the names below, sealed with a last member `"crc"`;
it is saved as `opendmd.json.new`, then the old one becomes
`opendmd.json.bak` and the new one takes its name, so a power cut leaves
the old settings or the new. It may be edited on a PC (the seal is then
ignored): a value the device would refuse over the API is kept as it was,
with a log line. A card from another display brings its settings; this
display's WiFi and secrets stay. A card without the file gets one, from
the settings in force, when it is first read. Copying the pack onto a
card, or formatting it, leaves no settings file: the next boot writes one.

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
| `hostname` | string, `"opendmd"` | name on the network (`opendmd.local`): 1-32 of `a-z 0-9 -`, no `-` at either end (lowercased) |
| `tz`, `tz_name` | strings, `"UTC0"`, `"UTC"` | POSIX time zone (`GMT0IST,M3.5.0/1,M10.5.0`, ≤ 64 characters); its display name (≤ 64) |
| `transition` | `dissolve` | `dissolve`, `wipe_left`, `wipe_right`, `wipe_down`, `wipe_up`, `blinds`, `push`, `fade`, `none` |
| `transition_ms` | 20..=2000, 400 | transition length |
| `auto_prepare` | `idle` | prepare copies by itself: `idle` (only while the panel is dark or paused), `always` (also slowly while a prepared GIF plays), `off` |
| `follow` | bool, false | Follow RePlayOS |
| `follow_host` | string | the RePlayOS box, `host` or `host:port` (≤ 64; the port 1-65535 in digits, else 400) |
| `follow_code_set` | bool | its code is saved (write it as `follow_code`, ≤ 64, one line) |
| `attract_set` | string or null | the playlist shown in RePlayOS's menus; null = the current one |
| `attract` | string, `""` | shown in the menus instead of a playlist: `""`, `"clock"`, `"text:<message>"` (≤ 200 characters, one line); anything else, `fx:` included, is `400` (the built-in effects are gone: for a Formula, make a playlist of it and choose it as `attract_set`). An `fx:` value stored by an earlier firmware reads as `""` (the playlist), with one log line at boot |
| `look` | `colour` | tint: `colour` (`color`), `amber`, `red`, `green`, `blue`, `white` |
| `glow` | `off` | phosphor glow: `off`, `short`, `long` |
| `look_formula` | `""` | a formula drawn over everything that plays, before the tint and glow: `""`, a bundled Formula by name (`@formula:<stem>`), one of yours on the card (`/formulas/<stem>.psi`, maybe `?a=…`; built again once the card is mounted) or inline, `@formula?psb=…` (≤ 2,048 B, else `413`: save it and set it by name; one that does not compile, is too heavy to draw over GIFs (12 ms estimated), or covers the GIF entirely is a `400` with the reason, nothing saved). How it goes over the GIF is the formula's own `look [blend] [amount]` line (`look screen 40`: its colour put on the GIF with the screen blend, covering 40 %; no line, or `look` alone: the formula as it is, over at 100 %); its name is its `name "…"` line. The web app offers Formulas that declare `look` |
| `boot_show` | `short` | start-up show: `off`, `short`, `full` (from the next boot) |
| `panel_sync` | `true` | refresh-synchronised frames: each frame waits (within its slack before the next) for a point in the panel's scan where writing it shows no seam; applied at once. `/api/status` `scan` counts seams either way. Formulas race the beam either way (`scan.beam`) |
| `panel_split` | `true` | with `panel_sync`: each frame's bottom half (rows 16-31) goes up a refresh after its top half. The panel lights rows r and r+16 together, top to bottom, so a frame shown whole is lit almost a refresh later at row 15 than at row 16 (moving edges step at the half line); split, the lighting time rises smoothly down all 32 rows. A frame whose next is due too soon for the second pass goes up whole; applied at once |
| `formula_jit` | `true` | the narrow JIT for Formulas on the ESP32 (`docs/research/psi-jit-esp32.md`): a Formula's pixel stage compiled to machine code at its first frame, checked against the VM there, then drawing the same bytes faster. `false` is the kill switch: every Formula plays on the VM (a kernel held is let go at its next frame). `/api/status` `now_playing.formula.jit` says which draws |
| `formula_arena` | `false` | a dev setting, in force from the next boot: a Formula's memory (its program and VM scratch, ink and paint planes, ~19 KB) from one block set aside at boot before the network starts, so it lands in the same SRAM pool every time (`display.buffers`; the A/B for the upload residual). Costs that block for the whole boot |
| `formula_keep` | `turn` | how long a Formula sent with `/api/play` stays on the panel: `turn` (one turn of the dwell, as any file), `1m`, `5m` or `hold` (until something else plays: another `/api/play`, `/api/skip` or Previous, a playlist change). Formulas in a playlist are not affected. From the next one sent |
| `card_speed` | `safe` | the card's SPI clock: `safe` (20 MHz), `fast` (40 MHz; at a card error, 20 MHz from then on for that card, changed in place with the transfer retried, so nothing restarts), `full` (API and bench only: always 40 MHz, never slowed; errors restart the player as a failing card does). 20 MHz is the default because the SD spec only promises SPI up to 25 MHz, and a marginal card at 40 MHz was measured returning wrong data with no error; at 20 MHz the card still reads several times faster than playback (tens of KB/s) or WiFi downloads (~300 KB/s) use it. Older firmware's `auto` reads as `safe` |
| `panel` | object | panel timing: `clk_phase` true (rising edge) or false, `latch_blanking` 1..=4 (1), `driver` `shiftreg`, `fm6126a`, `fm6124`, `icn2038s`, `mbi5124`, `dp3246`, `i2s_speed` 8, 16, 20 MHz (8), `min_refresh` 30..=240 Hz (60) |
| `panel_now`, `panel_next`, `panel_trial_s` | | what runs; what a restart would run, else null; seconds until the trial reverts, else null |

```sh
curl -s $DMD/api/config
```

### PUT /api/config

A partial update: only the fields to change, named as above (plus
`wifi_pass`, and `follow_code`, `""` clearing it). Applied at once:
brightness, look and glow repaint the panel; a transition applies from the
next file; a new `set`, `platforms` or `shuffle` rebuilds the playlist in
place; a new `hostname` at once (mDNS re-announces, status `mdns` says the
name it took; DHCP sends it from the next lease renewal). WiFi changes take
effect after a reboot.

Panel timing is tried: a `panel` object (any of its fields) runs for 60 s,
then reverts unless `"panel_confirm":true` keeps it (`false` undoes it; with
`panel`, it saves at once). `clk_phase` and `latch_blanking` apply live; the
others at a restart, and revert with one. `"panel":null` saves the defaults.

Reply: `{"ok":true, entries, index, index_ms, overrides, hidden,
"brightness", "max_brightness", "reboot_required", "panel", …}` (the
playlist fields as in "Fields that recur", the panel ones as above;
`reboot_required`: a WiFi or panel change waits for a reboot). With
`card_speed`, `card_speed_applied`: `now` (the clock changed in place),
`at_mount` (no card in), `restart` (the clock could not be changed in place: saved, and the device restarts after the reply).

Errors (nothing saved): `400` a value out of range or not one of the names
(the message says which), e.g. `brightness N is above the maximum M; raise
Maximum brightness first` (lowering `max_brightness` below the saved
`brightness` lowers it too); `404 {"error":"set not found"}` the playlist
does not open; `409` a confirm with no trial, or a taken `hostname` (below); `413` over 8 KB. With a
card in, a new `platforms` (or a `set` change with a filter kept) is checked
against the playlist that would be in force (a schedule rule's `set` if one
applies) before anything is saved: one that would leave it empty is a `400`
naming the filter; one whose playlist cannot be built at all is refused too
(`500`, nothing saved). Follow RePlayOS's game and attract playlists are not
checked up front; if a filter leaves nothing to play, Now says so and offers
Clear filter.

```sh
curl -s -X PUT $DMD/api/config --json '{"brightness":64,"transition":"wipe_left","transition_ms":300}'
curl -s -X PUT $DMD/api/config --json '{"set":"consoles","shuffle":true}'
curl -s -X PUT $DMD/api/config --json '{"panel":{"clk_phase":false}}'
```

### GET /api/name-check

`?name=kitchen`: does another device answer to `kitchen.local`? Asked over
mDNS (≤ 1.2 s): `{"name","valid","taken","by"}` (`by` its address;
`taken` null if it could not ask: an invalid name, setup mode, the network
busy; never for this player's own name). A taken `hostname` in `PUT
/api/config` is a `409` `{"error":"name taken: …","name","by"}`.

### POST /api/panel/test

The guided panel-timing test: the panel shows four test pictures, each
under A then B (2 s each, the letter in the bottom-right corner), and
nothing plays. `{"setting":"clk_phase"}` (A rising, B falling) or
`"latch_blanking"` (A as saved, B two more, at most 4; from 4, two fewer)
starts or restarts it; `{"stop":true}` ends it. It also ends after 120 s
or on any `PUT /api/config`; either way the saved timing and playback come
back. Nothing is saved (`panel_now` stays the saved values): save the pick
with `"panel_confirm":true`. Reply: `{"ok":true, "setting","a","b",
"left_s"}` (`a`, `b` as `panel`). `409` during a trial, with a change
waiting for a restart, with the panel off for the night, or `clk_phase`
on `mbi5124`/`dp3246`.

```sh
curl -s $DMD/api/panel/test --json '{"setting":"clk_phase"}'
```

### GET /api/wifi/scan

The networks in range (2.4 GHz, named, the strongest of each name), a
couple of seconds' scan: `[{"ssid","rssi","channel","auth","secure"}]`,
`rssi` in dBm. Empty in setup (access-point) mode: a scan would drop the
browser that asked. `504` if it takes over 12 s.

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
| GET | `/formulas/…` | one of the user's Formulas (`/formulas/<stem>.psi`, written with `PUT /api/files/formulas/<stem>.psi`) |
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

`size` is null without `sizes=1`: each costs a directory scan, minutes for
a folder of thousands. `404` no such directory.

```sh
curl -s "$DMD/api/files?dir=/sets&sizes=1"
```

### PUT /api/files/&lt;path&gt;

Write a file anywhere on the card, the body its bytes; directories are
created, a file replaced (the player moves off it first).

Reply: `{"ok":true,"path":"/…","bytes":N}`.

A playlist (`/filelist.*`, `/sets/*`) is installed as by
`PUT /api/sets/<name>`, with its reply; an overrides file (`.meta`)
rebuilds its playlist if in play. With a GIF, upload its prepared copy
too, or forget the old one.

Errors: `400` the path is not allowed (below); `408` the body stalled;
`503` busy, or the player did not let go of the file (retry).

**Write paths** (PUT, DELETE, move, `POST /api/cache/forget`) must stay on
the card and be spelled the one way: no empty, `.` or `..` component
(`gifs//x.gif`), none ending in a dot or a space (FAT drops them), and no
8.3 short name (`NAME~1.GIF`, `CONSOL~2`: it names a file whose long name
is something else, so its prepared copy and whether the player has it
open could not be told). Use the long name. The `400` says so. Reads by
path accept what FAT accepts.

```sh
curl -s -X PUT --data-binary @Super_Metroid.gif $DMD/api/files/gifs/Consoles/SNES_Super_Metroid_01.gif
```

### DELETE /api/files/&lt;path&gt;

Delete a file (the player moves off it first): a GIF with its prepared
copy, a playlist with its index; an overrides file rebuilds its playlist.
An empty directory under `/gifs/` too. Reply: `{"ok":true}`.

`?recursive=1` removes a whole directory, only under `/cache/` (and the
retired `/odm/`), else `400`; 4 s of work a request:
`{"ok":true,"path","removed":N,"done":bool}`, ask again until `done`.

Errors: `400` the path is not allowed (see PUT); `404` not there; `409`
the playlist in play, or a directory not empty; `503` the player did not
let go of the file (retry).

```sh
curl -s -X DELETE $DMD/api/files/gifs/Consoles/SNES_Super_Metroid_01.gif
curl -s -X DELETE "$DMD/api/files/cache?recursive=1"
```

### POST /api/files/move

Rename a GIF: `{"from":"/gifs/A/x.gif","to":"/gifs/B/y.gif"}`. Its prepared
copy follows; playlists and overrides are *not* rewritten (the web app
does that).

Reply: `{"ok":true,"from","to","copy":"moved"|"removed"|"none","smooth":bool,"failed":bool}`.

Errors: `400` not two different `/gifs/….gif` paths (spelled as PUT's
write paths); `404` no `from`; `409` `to` exists (a change of case is
fine); `503` the player did not let go of either file (retry).

```sh
curl -s $DMD/api/files/move --json '{"from":"/gifs/Other/a.gif","to":"/gifs/Arcade/ARCADE_A_01.gif"}'
```

## Playlists

A playlist is a text file, a card path a line: read at `/sets/<name>.txt`,
listed by `GET /api/files?dir=/sets`, picked by `PUT /api/config {"set":…}`,
deleted by `DELETE /api/files/sets/<name>.txt`.

### PUT /api/sets/&lt;name&gt;

Write `/sets/<name>.txt` (`.txt` added if missing), the body the file;
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

Rebuild the playlist in play's index (after the card changed some other
way). Reply: `{"ok":true, entries, "index":"built", index_ms, …}`.

```sh
curl -s -X POST $DMD/api/reindex
```

## Overrides and schedule

### GET /api/overrides

`?set=<playlist name>`. A playlist's per-GIF overrides, streamed:

`{"set","playlist":"/filelist.txt","overrides":[{"path", "never"?, "w"?, "dwell"?, "dwell_x"?, "loops"?}]}`

`never` hidden; `w` weight (Favourite 3, Play less 0.5); `dwell` seconds,
or `dwell_x` times the usual; `loops`. To change them, write the `.meta`
beside the playlist (`/sets/<name>.meta`, `/filelist.meta`), a line per GIF:
its path, then tab-separated `never`, `w=<n>`, `dwell=<s>` or `dwell=<k>x`,
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

`days` 1 (Monday) to 7 (empty or absent: every day); `set` and
`brightness` optional. The first matching rule wins; `to` ≤ `from` runs past
midnight. `night` (every day, beats every rule) has `mode` `"off"` (panel
dark) or `"dim"` (at `brightness`, default 8). Rules change what plays, not
the saved settings. Nothing applies before the clock is set from the
network.

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
The player makes them itself while the panel is dark or paused
(`auto_prepare`; `always` adds a slow drip while a prepared GIF plays); the web app makes them
faster and uploads them to `/cache/<Category>/<xx>/<name>.gif`
(`docs/cache-format.md`).

### GET /api/cache

How many of the playlist's files are prepared:

`{"total","prepared","state","count_ms","auto"}`: the playlist's length;
how many have a copy or play as they are (null until counted); `"counting"` or `"ready"` (poll
until ready); the last count's ms. `auto`, the player's own preparing:
`{"mode","state","reason","policy","session","failed","avg_full_s","avg_drip_s","eta_s","current"}`:
`mode` = `auto_prepare`; `state` `idle`, `running`, `paused`, `done`;
`reason` it is not running (`off`, `playing`, `gif on screen` (an
original plays), `late frame` (held off a while after a visibly late
frame), `low memory`, `busy`, `transition`,
`reindex`, `no playlist`, `card error`);
`policy` `full` or `drip` (pace); `session` copies since boot; `failed`
GIFs it cannot prepare; `avg_*_s` s per file per pace; `eta_s` s left at
the full pace; `current` the GIF in hand.

```sh
curl -s $DMD/api/cache
```

### GET /api/cache/list

Every prepared copy, streamed: `{"paths":["/cache/…"],"count":N}`.
Subtract it from a playlist for what is left to prepare.

```sh
curl -s $DMD/api/cache/list | jq .count
```

### GET /api/cache/failed

The GIFs with no copy, streamed:
`[{"path":"/gifs/…","reason":"image truncated"}]`, `[]` if none. Reason
`more than 256 colours…`: it plays as it is (done).

```sh
curl -s $DMD/api/cache/failed
```

### POST /api/cache/failed

Record a GIF the browser could not prepare either (or one that plays as it
is), so the player skips it too: `{"path":"/gifs/….gif","reason":string}` (kept to one line, ≤ 200
characters). Reply: `{"ok":true,"path","added":bool}` (false: it was
there). `400` not a `/gifs/….gif` path.

```sh
curl -s $DMD/api/cache/failed --json '{"path":"/gifs/Other/broken.gif","reason":"image truncated"}'
```

### DELETE /api/cache/failed

Forget the whole list, so replaced GIFs are tried again. Reply:
`{"ok":true,"cleared":bool}` (false: none).

```sh
curl -s -X DELETE $DMD/api/cache/failed
```

### POST /api/cache/forget

Delete one GIF's prepared copy (after replacing the GIF):
`{"path":"/gifs/….gif"}` (on the panel, the player moves on first).
Reply: `{"ok":true,"path","removed":bool}`. `400` not a `/gifs/….gif` path.

```sh
curl -s $DMD/api/cache/forget --json '{"path":"/gifs/Consoles/SNES_Super_Metroid_01.gif"}'
```

## Follow RePlayOS

With `follow` on, the panel follows a RePlayOS box: the game's art while a
game runs, the attract playlist (or `attract`) in the menus. Set with
`PUT /api/config` (`follow`, `follow_host`, `follow_code`, `attract_set`,
`attract`); its state is `/api/status` `context`.

### POST /api/follow/test

Ask the box for its version and status, saving nothing:
`{"host"?: string, "code"?: string}` (absent: the saved ones; an empty body
is fine). Up to ~20 s.

Reply: `{"ok","reachable","version","status","view","system","game","error"}`;
`status` is the box's HTTP status (401: wrong code). An early failure has
only `ok`, `reachable`, `error` (and `version`).

```sh
curl -s $DMD/api/follow/test --json '{"host":"192.168.1.20","code":"1234"}'
```

## Network

The player joins the WiFi network in its settings. With none, or after
failing to join twice, it raises an open network, `OpenDMD-Setup`, as a
captive portal (unknown paths redirect to `/`), and restarts every 10
minutes to try again.

| method | path | what |
|---|---|---|
| GET | `/` | the web app (in setup mode with no good web app in flash, the setup page) |
| GET | `/setup` | the built-in setup page: join a network |
| GET | `/hotspot-detect.html`, `/generate_204`, … | captive-portal probes: a redirect to `/` in setup mode, else 404 |

Over USB, the browser installer sets it with Improv Serial instead (the
player joins at once, no reboot). Over HTTP, set the network with `PUT /api/config`, then reboot:

```sh
curl -s -X PUT $DMD/api/config --json '{"wifi_ssid":"MyNet","wifi_pass":"secret"}'
curl -s -X POST $DMD/api/reboot
```

## Security

This keeps out casual use (a guest, a script, another web page in your
browser), not someone on the same WiFi: it is plain HTTP.

- A `POST`/`PUT`/`DELETE` whose `Origin` is another host than its `Host`
  is a `403` (no `Origin`, as from curl, is fine).
- **API key** (Settings → Device → Security): with `api_key` set (`PUT
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

The web app is one file in the player's flash, a www pack
(`docs/www-pack.md`): `opendmd-web/www.pack` from `make web`,
`dist/opendmd-<version>-www.pack` in a release. Served at `/…` and `/www/…`.

### GET /api/www

`{"version","bytes","entries","ok","error","staged"}`: the pack's version
(`git describe` of its build), size, file count, whether it verified, why
not (erased, damaged, an update cut short, being written), whether this
boot's last update was staged on the card (null: none, or refused at its
header).

```sh
curl -s $DMD/api/www
```

### PUT /api/www

Write a new pack (the whole file). Its header is checked before anything
is erased: not a pack, an unknown format, or a length not the body's is a
`400` at once, and the app in flash stays. With a card, the pack is staged
there and read back (size, CRC; failed reads retried): a `408` (cut short),
`422` (bad CRC) or `500` (unreadable) leaves the old app. Then it is
written to flash (~7 s for 800 KB) and served at once, no reboot. If that
write fails (`500`), pages are `503` and the next restart writes the pack
from the card, or push again. Without a usable card, a `408`/`422` leaves
pages `503` until a good pack is written.

Reply: `GET /api/www`'s object (`staged` false: no card staging). `411` no
length; `413` too big. Any good pack is accepted, older or newer: check the
version `GET /api/www` reports (`make www-push HOST=…` does).

```sh
curl -s -X PUT --data-binary @opendmd-web/www.pack $DMD/api/www
curl -s $DMD/api/www
```

## Firmware update

A signed bundle (`dist/opendmd-<version>-<variant>.odfw`, `make fw-release`) is
staged on the card (one is needed), checked and installed (`docs/ota.md`);
`make fw-ota HOST=…` does it.

### GET /api/firmware

`{"version","build","variant","flash_size","flash","pending","last","log"}`: the
running version (a bundle's, and `/api/status`'s; `build` its image
descriptor, `"<version> <variant>"`), the flash layout it is built for
(`variant`: `4mb` or `8mb`) and the flash chip's real size in bytes
(`flash_size`; a 4mb build on an 8 MB board reports 8388608), the clock and
read mode the flash runs at (`flash`: `{"mhz": 40|80, "mode": "dio"|…}`,
read back from the chip), a staged
update not yet installed (or null), the last
line of the card's `/update/log.txt` and its last ≤ 1 KB of whole lines
(`log`, the firmware's and the updater's; both null without a card or log).

```sh
curl -s $DMD/api/firmware
```

### PUT /api/firmware

The bundle as the body (`411` no length, `413` too big). Header, the flash
layout (the bundle's must be the player's `variant`), size, SHA-256,
Ed25519 signature and the image's app descriptor are checked: a failure is
a `422` with the reason, nothing staged; cut short `408`; no card `503`.
Another key's bundle, or one for the other layout, is refused once the
140-byte header is in. Any validly
signed bundle is accepted, older ones too. Then
`{"ok":true,"version":"…","rebooting":true}`; the panel is dark ~45 s from
the end of the upload to playback while it installs.

```sh
curl -s -X PUT --data-binary @dist/opendmd-0.4.0-4mb.odfw $DMD/api/firmware
```

## Live items

Paths beginning with `@` are drawn, not read from the card: for
`POST /api/play`, a playlist line, `attract`. Options follow `?` as
`key=value` pairs joined by `&`; unknown keys and bad values are ignored
(the default stays). Colours are `RRGGBB` (or `%23RRGGBB`, `#RRGGBB`).

**`@clock`** — the time (local, from the network; `--:--` until set).
`style=big|small` (default big), `seconds=1`, `date=1` (the date under it),
`h12=1` (12-hour), `color=RRGGBB`. One loop is one second.

```sh
curl -s $DMD/api/play --json '{"path":"@clock?style=small&seconds=1&color=ff8000"}'
```

**`@text`** — scrolling text. `msg=<url-encoded>` (≤ 200 characters, ASCII
shown), `speed=<ms per pixel>` (20..=1000, default 40),
`font=big|normal|small` (default big), `color=RRGGBB`, `scroll=0` (still,
centred, if it fits). A loop is one pass across the panel.

```sh
curl -s $DMD/api/play --json '{"path":"@text?msg=Game%20over&speed=30&color=00ff00"}'
```

**`@formula`** — a formula (the app's Formulas), **compiled**: the player
never compiles. `psb=<the compiled Formula, base64url, no padding>` (what
the web app's pkg-psi `program(src)` makes, or `psi build`), `a=`, `b=`,
`c=` (sliders), `fps=50|100|25` (default:
the formula's `fps` line, else 50; as a look over the GIF the line is not
used). A heavy one is shown at fewer frames a second; status
`now_playing.formula` has `frame_us`, `frame_ms`, `asked_ms`,
`beam_us` (the raced frame time on the panel's refresh, µs; 0 while
its frames are not raced, see `scan.beam`),
`compile_us` (the load's time, µs), `still` (the still cache's bytes) and `jit`:
`{"on":true,"bytes":N}` while the JIT's kernel (N bytes of code) draws
its pixel stage, else `{"on":false,"why":"…"}` (e.g. `it has layers`,
`it reads the picture under it`, `it draws (ink)`, `off (setting)`,
`being checked`), and `render_us_total`/`renders`: the playing
formula's render µs in all (wrapping) and renders since boot, whose
differences give a stretch's exact mean (`frame_us` is a running mean of
the last few frames). The whole inline path is at most 6,144 B;
`name=<url-encoded>` (≤ 60 characters) is its name for display.

**By reference** (what playlists and the look use, so a Formula's length
never matters there): `@formula:<stem>[?a=&b=&c=&fps=]` plays a bundled
Formula (its file's stem in `opendmd-web/formulas/`, compiled into the
firmware: `endless-road`, `big-clock`, …); a card path ending `.psi`,
`/formulas/<stem>.psi[?a=…]`, plays one of the user's from the compiled
file beside it, `/formulas/<stem>.psb` (16 KB at most), which the web app
writes whenever it saves the `.psi`. A name that is not bundled, or a
Formula with no compiled file, is a `404`; one compiled for another
version of the player is a `400` (`/api/status` `psi_abi` is the version
this player takes; the web app prepares its Formulas again for it). Its
title is the compiled Formula's name (its `name "…"` line).

```sh
curl -s $DMD/api/play --json "{\"path\":\"@formula?psb=$(psi build my.psi --base64)\"}"
curl -s $DMD/api/play --json '{"path":"@formula:endless-road"}'
curl -s $DMD/api/play --json '{"path":"/formulas/my-road.psi?a=0.4"}'
```

**Retired: `@fx:`.** The built-in effects (`@fx:tunnel`, `@fx:rotozoom`,
`@fx:starfield`, `@fx:wire`) and the old Formula path `@fx:psi?src=…` are
gone: they are Formulas now. `POST /api/play` answers `415` for any `@fx:`
path. In a playlist (`/filelist.txt`, `/sets/*.txt`) such a line is skipped,
with one log line each time the playlist is read (`playlist: skipping
"<line cut to 48 chars>…": the built-in effects are gone (Formulas replace
them)`); the rest of the playlist plays.

**`@boot`** — the start-up show. `len=short|full|card` (`card`: the status
card), and for it `v=<version>`, `ip=<address>`, `ap=<network>`,
`note=<text>` (≤ 32 characters each).

```sh
curl -s $DMD/api/play --json '{"path":"@boot?len=full"}'
```

Also `@power-on` (the power-on effect) and `@card:<title>|<subtitle>` (a
title card, 3 s).
