# The www pack

The web app (`opendmd-web/www` after `make web`) ships to the device as one
file, a **www pack**, written into the `www` flash partition
(`firmware/partitions.csv`: data/`esphttpd`, 0x2C0000, 1,280 KB). At boot the
firmware memory-maps it and serves every file from the mapping, through the
flash cache: no card, no heap per file, no filesystem.

There is one implementation of the format: `opendmd_core::www_pack`
(`opendmd-core/src/www_pack.rs`, no dependencies). The packer
(`opendmd-pack www`) and the firmware (`firmware/src/www.rs`) both use it,
and its unit tests round-trip a pack and check that damage is caught.

## Layout

Little-endian throughout; offsets are from the start of the pack.

```
offset  size  field
     0     4  magic "ODWP"
     4     2  format version: 1
     6     2  header length: 64
     8     4  total length of the pack, header included
    12     4  CRC-32 of bytes [64, total length)
    16     2  entry count
    18     1  version string length (<= 44)
    19     1  reserved, 0
    20    44  version string, UTF-8, zero-padded
    64        entry table: count x 16 B, sorted by path
              path strings, back to back (UTF-8, no terminators)
              zero padding to a multiple of 4
              file data, each file starting on a 4-byte boundary
              (zero padding between files; none after the last)
```

An entry (16 B):

```
offset  size  field
     0     4  path offset
     4     4  data offset
     8     4  data length
    12     2  path length (> 0)
    14     2  flags: bit 0 = the data is gzip; other bits must be 0
```

- **Paths** are what the browser asks for, without the leading slash:
  `index.html`, `js/smooth.js`, `pkg-art/opendmd_art_web_bg.wasm`. Entries
  are sorted by the paths' bytes, strictly (no duplicates), so a lookup is a
  binary search: six comparisons for the ~40 files of today's app.
- **The version string** is `git describe --always --dirty --tags` of the
  build (`make web` writes it to `www/version.txt` too, which is not itself
  packed). The firmware reports it as `www_version` in `/api/status` and as
  `version` in `GET /api/www`. 44 bytes is room for
  `v0.10.12-123-g0123456789ab-dirty` and then some; the packer refuses a
  longer one.
- **The CRC** is CRC-32/ISO-HDLC, the one zlib, gzip and PNG use
  (`crc32(b"123456789") == 0xCBF43926`), over everything after the header.
  The header itself is covered by its own checks (magic, format, lengths)
  and by every entry's bounds check.
- **Alignment**: 4 bytes for file data is all the flash cache and `memcpy`
  want. The header is not padded to anything larger.
- **Content type** is not stored: the firmware derives it from the path's
  extension (`files::mime_for`, the table it always used), and the cache
  policy from the path (`files::cache_for`).

## What goes in

`opendmd-pack www <dir> --out <pack> (--version <v> | --version-file <f>)
[--max-bytes N]`, run by `make web`:

- every regular file under `<dir>`, dotfiles skipped;
- where `<file>.gz` sits beside `<file>` (`make web` gzips every `.html`,
  `.js`, `.css`, `.wasm`, `.svg` and `.json`, `gzip -9 -n`), the entry
  `<file>` holds the **`.gz`'s bytes** with the gzip flag, and the plain
  file is not stored; a lone `<file>.gz` is stored as `<file>`, gzipped;
- anything else as it is (today the font and its licence, `fonts/`);
- the `--version-file` itself is left out.

The HTTP API reference is not in the pack: it is docs/api.md on the project
site (`dev/api.html`, and `dev/api/v<version>.html` for each release), which
Settings → Device → About links (14.5 KB of flash, gzipped, until 2026-10-23,
when it was `api.html` with a size line of its own).

The firmware sends a gzip entry with `Content-Encoding: gzip`, without
looking at `Accept-Encoding` (every browser accepts it), and a `Content-Length` of the stored bytes.

The packer prints `pack: N B of 1,280 KB (P%)` and refuses a pack larger
than `--max-bytes` (default and `make web`'s: 1,310,720, the partition),
writing nothing. It then reads the file back through the reader and checks
it. `opendmd-pack www-info <pack>` prints the header and every entry and
verifies the CRC. `tools/web-size.py` prints the same `pack:` line under
its gzip budgets (which are about load time over WiFi, not flash).

## On the device

- **Boot** (`www::init`, on the main task before the card is mounted): read
  the 64-byte header with `esp_partition_read`; if it parses, map exactly
  `total length` bytes with `esp_partition_mmap(…,
  ESP_PARTITION_MMAP_DATA, …)`; parse (every entry bounds-checked, order
  checked) and compute the CRC over the mapped bytes in one pass — no buffer,
  the table is in flash. Logged as `www: pack '<version>' verified, N
  entries, B B, mapped at 0x…, CRC pass M ms`. A missing, erased or damaged
  pack is logged, reported in `/api/status` (`www.ok` false, `www.error` the
  reason) and every web app request is answered `503` with the reason in
  plain text; the API, the player and `/setup` carry on (in AP mode `/`
  shows the setup page instead, so a board can always be put on WiFi).
- **Serving** (`www::serve`, on the stream task like every file): the
  mapping's lock is held for the whole response; the file goes out in 1 KB
  slices, each copied from the mapping into a buffer on the stream task's
  stack (CPU0) and then sent. Not the mapping itself: lwip copies a send
  buffer on its tcpip task, and that copy running on CPU1 would pull the
  pack through the flash cache the display loop runs from. The tcpip task
  is pinned to CPU0 as well (`CONFIG_LWIP_TCPIP_TASK_AFFINITY_CPU0`), which
  is what took a flat-out UI download from +66 to +8 deadline misses per
  90 s.
- **Updating** (`PUT /api/www`, `www::receive`, on the stream task): the
  whole pack as the body, `Content-Length` required (411 without, 413 over
  the partition). The first 64 bytes are read and parsed before anything is
  erased — not a pack, or a header whose total length differs from the
  body's, is a 400 (answered at once, without waiting for the rest of the
  body) and the app in flash stays. Then:
  - **With a card** (the usual case), the whole pack, header first, is
    received onto the card, `/update/www.pack.part` (the GIF upload path:
    a 2 KB buffer, one open file), and **read back whole before anything
    in flash is touched**: its size, and its CRC against the header's,
    all while the old pack stays mapped. An upload cut short (408), a
    client that went away, a card write that failed (500), a CRC that
    does not match (422) or a card that will not read the file back (500,
    `reading the staged pack from the card at byte N failed 4 times: …;
    the web app in flash is unchanged`) leaves **the web app in flash
    exactly as it was**, still served. Only a staged pack that read back
    good is written to flash, from the card (~3 s for 800 KB, most of it
    the erase pauses).
  - **Card reads are retried.** A card can fail one read with a data CRC
    error and read the rest fine, so every
    read of the staged file, in the check and in the copy, is tried up to
    `config::WWW_CARD_READ_TRIES` (4) times, 100/200/300 ms apart, the file
    opened again each time (FATFS remembers a disk error in the open file
    and fails every later call on it). Only a read that fails every time
    counts as one card error for the card's health (`sdhealth`).
  - **If the copy still fails** after the erase (the card gave up between
    the check and the copy, or a flash write failed), the whole write is
    tried once more from the start; if that fails too, the reply is a 500
    saying so, pages are 503, and the checked pack **stays on the card**.
    At the next boot, once the card is mounted and `www::init` has found no
    pack in flash, `www::restore_from_card` checks the staged file again
    (size, CRC) and writes it the same way (logged: `www: pack '…' written
    from the card in N s; the web app is back`). A staged file found while
    flash holds a good pack is stale and deleted; one that is not a good
    pack is deleted; one the card will not read stays for the next boot.
    A new push replaces it either way. The boot task does this before it
    starts the network, so WiFi comes up that much later (~9 s for 877 KB).
  - **Without a card** (none, one that cannot be read, or one where
    `/update/www.pack.part` cannot be created), the body is written to
    flash as it arrives: a failure part-way leaves no web app
    until a good pack is written (the setup page and the API carry on).
    The log says which it was: `www: staging on the card (…)` only once
    the staging file exists, otherwise `www: no card staging (<reason>):
    writing to flash from the upload, the old web app is replaced during
    the transfer`; the reply's `staged` is false, and `make www-push`
    prints a warning.
  - **The header is written last** either way. Under the mapping's lock:
    unmap; erase sector 0 and the sectors ahead of each piece
    (`esp_partition_erase_range`, then a 20 ms pause so the display loop
    runs: an erase stops both CPUs' flash caches) and write the body from
    offset 64 (`esp_partition_write`), leaving the header's 64 bytes erased;
    read the body back from flash and compute its CRC; only if it matches
    the header's, write the 64-byte header into the erased bytes; then
    parse, CRC-check and map again. So **a pack header in flash always
    means a whole pack whose CRC verified from flash**: a reset or a
    failure during the write leaves an erased header ("no web app"), never
    a header over a half-written body. The window in which the device has
    no web app is the flash write itself (~3 s from the card), not the
    network transfer.

  The reply is `GET /api/www`'s object; a pack that does not verify after
  writing is a 422, one cut short a 408 (or nothing, if the client went
  away), a copy from the card that failed twice a 500. 782 KB takes ~7 s over WiFi plus the flash write.
- **`GET /api/www`**: `{"version","bytes","entries","ok","error","staged"}`;
  the same object is `www` in `/api/status`, and `PUT /api/www`'s reply.
  `staged`: whether this boot's last update attempt was staged on the
  card (true, also for one refused after staging), written from the
  upload (false), or none yet (null; also after an attempt refused at its
  header, so it never describes an earlier push).

A response in flight and an update never overlap: both run on the one
stream task, one job at a time, and the lock makes that hold even if the
stream task is ever absent (then both run on the httpd task, also one at a
time). A request that arrives during an update waits in the stream queue
and is served from the new pack once it is mapped (measured: a page load
started 2.5 s into a push rendered 5.2 s later); only a third request, with
two already waiting, gets `503` + `Retry-After: 1` after 2 s, as behind any
upload.

## Changing the format

Bump `FORMAT` in `www_pack.rs`. A firmware that meets a pack of another
format reports `unsupported pack format N` and answers 503 until a pack it
understands is written, so ship the firmware and the pack together (the
flasher and `make fw-flash` always write both).
