# OpenDMD Connect protocol, version 1

Remote control of a panel from away from home: a browser or app (a
**controller**) talks to the panel through a relay (the **server**: a
Cloudflare Worker with one Durable Object per panel). The panel makes the
only outbound connection; nothing on the home network is opened. The server
relays and pre-filters, but it can never command a panel: every command is
signed by a controller key the panel itself paired, and the panel checks
each one against its own list.

This document is the reference. The implementations are:

| Where | What |
|---|---|
| `opendmd-core/src/connect/` (feature `connect`) | the panel's checks and the controller's encoding, in Rust (`proto.rs`, `keys.rs`, `pair.rs`, `xfer.rs`, `codec.rs`, `json.rs`; `ws.rs` is the WebSocket client) |
| `opendmd-core/tests/connect-vectors.json` | the shared test vectors: every implementation checks itself against them |
| `tools/connect/` | an independent Python implementation (`odc.py`), a mock server and a mock panel, and the round trip (`make connect-mock`) |
| `firmware/src/connect/` (feature `connect`) | the panel's client: TLS, registration, the session policy, the commands carried out (`make emu-connect` runs it in the emulator against the mock server) |

The firmware's client, the Worker and the web controller are built on this
version; none of it changes the LAN API, which stays the only full tier. A
panel is never updated, wiped, reconfigured or restarted remotely.

## 1. Transport

- WebSocket over TLS. The panel connects to `GET /v1/panel`, a controller
  to `GET /v1/ctl`.
- Subprotocols: `odc<N>` (the protocol version) and `d.<dev>` (the panel's
  Connect id, section 3). The id is never in a URL. The server answers with
  the `odc<N>` it chose.
- A controller's `Origin` must be `https://opendmd.dev` or absent (an app);
  anything else is refused (HTTP 403) before the upgrade.
- An id whose tag does not check (section 3) or that the server does not
  know is refused (HTTP 404) before any object wakes.
- The panel's TLS input cap is 4 KB: a larger record ends the session.
- Keepalive: the panel sends a WebSocket ping every 90 s and reconnects
  after two unanswered. Browsers send the text `ping` every 60 s, answered
  `pong` by the runtime.

### 1.1 Frames

**Text frames** are one JSON object each, with a type `t`: at most
**1,024 bytes** for any sent to the panel or by a controller (the panel's
receive buffer), at most **2,048 bytes** for those the panel sends (an
`ack` with a full `controllers` result is about 1.3 KB), which the server
and controllers must take. A message of an unknown type is ignored; unknown fields are
ignored.

**Binary frames** start with one type byte:

| Type | Layout | Limits |
|---|---|---|
| `0x01` signed command | key id (8) ‖ Ed25519 signature (64) ‖ body | body ≤ 700 bytes (UTF-8 JSON) |
| `0x02` transfer chunk | transfer id (4, big-endian) ‖ offset (4, big-endian) ‖ data | data 1 to 1,024 bytes |

A binary frame of another type, or a command frame shorter than 73 bytes,
is dropped (no `ack`: there is no key id to answer).

### 1.2 JSON, strictly

Every party reads JSON the same strict way, so a message can never be read
two ways:

- a **duplicate key** in any object is an error (also when one spelling
  uses an escape, `"c"` and `"\u0063"`);
- numbers are **integers only**, `-?(0|[1-9][0-9]*)`, magnitude at most
  2^53 - 1; no fraction, no exponent, no `-0`;
- strings are valid UTF-8, with no raw control character, no lone surrogate
  and no U+0000;
- nesting at most 4 deep; the top level is an object; nothing but
  whitespace after it.

### 1.3 Encodings

- **Hex**, lowercase only, for every binary value inside JSON: key ids,
  public keys, signatures, MACs, nonces, hashes, `sess`. Uppercase is
  refused: one spelling per value.
- **Base32** (RFC 4648's alphabet, lowercase, no padding) for the values a
  person or a QR code carries: `dev` (16 bytes, 26 characters) and the
  pairing secrets. Canonical only: the unused low bits of the last
  character are zero. (A pairing secret read from a link is lowercased
  first.)

## 2. Versions

- The version is `N` in the `odc<N>` subprotocol: decimal, no leading zero,
  1 to 999. **The server accepts the current version and the one before
  it**, and picks the newest it accepts among those offered. Today only 1
  exists; the mechanism ships anyway, because a panel cannot be updated
  remotely and the server must go on speaking to the ones in the field.
- Only older versions offered: the server upgrades, then closes **4426**
  with the lowest version it accepts as the reason. The panel shows
  "update needed" and retries daily; a controller says "update this panel
  from your home network".
- Only newer versions offered: close **4406** with the newest it accepts
  (a server behind a panel; not expected in practice).
- No `odc<N>` at all: HTTP 400.
- The body of every signed command carries `v`, which must equal the
  connection's version.

`GET /v1/health` answers `{"ok":true,"v":1,"min_v":1}` and nothing per
panel.

### Close codes

| Code | Meaning | Reason text |
|---|---|---|
| 4426 | too old | the lowest version accepted |
| 4406 | too new | the newest version accepted |
| 4429 | stay away | minutes to wait |
| 4403 | refused identity: unknown panel, wrong panel secret, unknown controller key or bad attach signature (all alike), a guest with no open window, a removed key | none |

## 3. Identities

| Thing | Made by | Kept | Notes |
|---|---|---|---|
| Connect id `dev`, 16 bytes | the server, at registration | panel NVS; controllers; the object's name | 10 random bytes ‖ the first 6 bytes of HMAC-SHA256(`ID_KEY`, `"opendmd-connect-v1 dev\0"` ‖ the 10 bytes). The server checks the tag before anything else, so a made-up id costs nothing |
| Panel secret, 32 random bytes | the panel | panel NVS; its SHA-256 at the server | proves "I am this panel" in `hello` |
| Controller key, Ed25519 | each browser or app | WebCrypto non-extractable key in IndexedDB; Keychain | key id `k` = the first 8 bytes of SHA-256(public key) |
| Controller list, at most 8 | the panel | panel NVS, one blob of at most 563 bytes; the public keys and flags copied to the server | labels stay on the panel |
| Panel name, at most 32 characters | a controller, by signed command | panel NVS; copied to the server | invisible characters stripped |

A controller entry: public key, flags (bit 0 `can_add`), label (at most 24
UTF-8 bytes), the date added (Unix seconds, 0 without a clock), and the key
that added it (none for a key paired on the panel). A public key must be a
valid curve point and not of small order. A key paired on the panel (LAN
link or QR) gets `can_add`; a key added by another controller never does:
one level only.

Labels and names are cleaned before they are stored: control characters
and the invisible ones that can disguise text (zero widths U+200B to
U+200F, line and paragraph separators U+2028 and U+2029, bidi controls U+202A to U+202E and U+2066 to U+2069, U+2060 to
U+2064, U+00AD, U+FEFF, variation selectors, tags, Hangul fillers) are
removed, then the text is trimmed; empty or too long after that is refused.

The NVS blob: `K`, version 1, the count, then per entry the public key
(32), flags (1), the date (4, big-endian), the adder's key id (8, zeros for
none), the label's length (1) and the label. A blob with trailing bytes,
more than 8 entries, unknown flags, a bad key or label, or a duplicate is
refused whole.

## 4. Messages

### 4.1 Panel and server

| Direction | Message | Fields |
|---|---|---|
| panel → server, HTTPS, once | `POST /v1/register` | `{"v":1,"secret_hash":"<hex SHA-256 of the secret>"}` → `{"dev":"<base32>"}` |
| panel → server | `hello` (first message) | `v`, `secret` (hex), `fw`, `sess` (12 random bytes, hex, new at every connect), `caps {send, win}` |
| panel → server | `keys` | `keys: [{k, pub, f}]`, after `hello` and after any change |
| panel → server | `info` | `name`, after `hello` and after a rename |
| panel → server | `pair-open` / `pair-close` | `pair_id` (hex), `ttl` (seconds) |
| panel → server | `status` | section 7; only while watched |
| panel → server → one controller | `ack` | section 5.4 |
| panel → server → one controller | `credit` | `id`, `upto`, `k` (the key that started the transfer); the last one also `done` and `name`, or `err`, section 8 |
| panel → server → one guest | `paired` | `g`, then `k` and `mac2`, or `err` |
| panel → server | `forget` | the last message under an identity being dropped: the server deletes its record |
| server → panel | `watch` | `on`: whether any controller is attached |
| server → panel | `pair` | a guest's `pair`, with `g` (the guest's slot) added |
| server → panel | binary `0x01`, `0x02` | relayed from controllers |

**The guest slot `g`.** The server gives each guest a slot `g`, an
integer it adds to the guest's `pair` (and to `add-req`), sends in
`guest-ok`, and routes `paired` (and `add-ok`) by. The server **must**
assign `g` in 1 to 255, distinct among the guests of one panel: panels up
to 1.1.0-beta.4 read it as one byte and drop a `pair` with a larger one as
malformed (no `paired` ever comes). Panels after 1.1.0-beta.4 take any
integer 0 to 2^31 - 1 and echo it unchanged in `paired`; anything else
(negative, larger, not an integer) is still malformed and dropped.

The server hashes `hello.secret` and compares it with the stored hash;
else it closes 4403 and the panel waits 15 minutes (Settings says to set
Remote control up again). A newer panel connection replaces an older one.

An id the server does not know at the upgrade (HTTP 404) is handled by
whether anyone could know it. A panel with no controllers (a registration
that never said `hello` within 10 minutes is deleted) registers again by
itself, at most three times a day. A panel with controllers treats it as
a refusal, like 4403: retried every 15 minutes, nothing erased, so a
server that lost its records (a bad key deploy, lost storage) cannot wipe
pairings; the owner chooses Forget on the LAN to start again. A panel that forgets its
identity sends `forget` if it is online; offline, the old record stays at
the server, never used.

### 4.2 Controller and server

| Direction | Message | Fields |
|---|---|---|
| server → controller | `challenge` (first message) | `nonce`, 16 random bytes, hex |
| controller → server | `attach` | `k`, `sig` = Ed25519 over `"opendmd-connect-v1 attach\0"` ‖ `dev` (16 bytes) ‖ nonce (16 bytes) |
| server → controller | `attached` | `sess` (the panel's, or `null` if offline), `online`, `name`, `fw` |
| controller → server | `guest` (instead of `attach`) | `pair_id` or `invite_id`: pairing only, section 6 |
| server → guest | `guest-ok` | `g` |
| server → controllers | `panel` | `online`, and the new `sess` when it comes back |
| controller → server | `invite-open` / `invite-close` | `invite_id`, `ttl` |
| guest → server → inviter | `add-req` | `pub`, `label`, `mac`; the server adds `g` |
| inviter → server → guest | `add-ok` | `g`, `k` |
| controller → server → panel | binary `0x01`, `0x02` | section 5, section 8 |

An unknown key id and a bad attach signature get the same close (4403). A
guest is admitted only with the `pair_id` or `invite_id` of an open window,
at most two at a time, for at most 60 s. A controller's frames under
another paired key's id are refused (`unknown-key`).

`POST /v1/find {"pair_id"}` → `{"dev"}` (404 if no window is open): how a
controller that has only the pairing secret finds the panel. The mapping
lives for the window's 5 minutes and goes when the pairing succeeds or the
panel closes the window.

## 5. Signed commands

### 5.1 The body

```json
{"v":1,"dev":"<base32>","sess":"<24 hex>","n":7,"c":"brightness","a":{"v":96}}
```

- `v`: the connection's version. `dev`: the panel's id. `sess`: the
  panel's nonce for this connection (from `attached` or `panel`). `n`: this
  key's counter, 1 to 2^53 - 1, above the last one it used this session.
  Tabs and pages of one browser share its key and can race, so a
  controller uses `n` = max(last used + 1, `Date.now()` * 1024 + seq) and,
  on `replay`, signs again once with a fresh `n`.
  `c`: the command. `a`: its arguments, an object (may be left out when
  there are none).
- Member order and whitespace are free: the signature is over the bytes.

### 5.2 The signature

Ed25519 (RFC 8032, verified strictly: no small-order keys, canonical `S`)
over `"opendmd-connect-v1 cmd\0"` ‖ **the body's bytes as sent**. No
canonical JSON form exists or is needed: the panel verifies the bytes,
then parses them. A body re-serialised by anyone in between fails.

### 5.3 What the panel checks, in order

1. The frame: type `0x01`, at least 73 bytes (else dropped); body at most
   700 bytes (else `too-big`).
2. The key id is on the panel's list (`unknown-key`).
3. The panel's rate, 8 commands a second (`rate`), before the signature
   check, so a flood costs no 30 ms checks.
4. The signature (`bad-sig`).
5. The body parses as strict JSON (`bad-arg`); `n` is an integer of at least
   1 (`bad-arg`); `v` is the connection's (`bad-arg`).
6. `dev` is this panel's and `sess` this connection's (`stale-session`).
7. `n` is above this key's last `n` this session (`replay`); the counter
   then advances, whatever follows.
8. The key's rate, 4 commands a second (`rate`).
9. `c` is in the allow-list (`not-allowed`) and its arguments pass
   (`bad-arg`, `too-big`, `quota`, `not-allowed`).

Counters live in RAM, per connection: no flash write per command and no
clock. A reconnect makes a new `sess`, which voids everything signed
before it; the controller signs again and retries once. A key removed and
added again keeps its counter for the session. A session keeps counters
for at most 16 keys; past that it answers `busy` and the panel reconnects.

The server runs steps 1, 2 and 4 against its copy of the keys before
forwarding, and rates of its own (4 a second per controller, 8 per panel),
so strangers and floods never reach the panel. The panel still trusts only
its own list. A command for an offline panel is answered `busy` at once:
there is no queue.

### 5.4 `ack`

```json
{"t":"ack","k":"<hex>","n":7,"ok":true}
{"t":"ack","k":"<hex>","n":7,"ok":true,"r":{"paused":true}}
{"t":"ack","k":"<hex>","n":7,"err":"replay"}
{"t":"ack","k":"<hex>","err":"bad-sig"}
```

`n` is left out when it could not be read. The server delivers an `ack`
to the sockets of key `k`.

**Acks are not authenticated.** The panel does not sign them, so a
hostile server can fake an `ok`, a `controllers` list or a `playlists`
or `formulas` result, or drop a real one; it still cannot make the panel do anything.
And a signed command the server withholds stays valid until that key's
next command in the same session (or the session ends): the server can
delay a command, never forge or repeat one. Both are accepted for P1.

Errors: `bad-sig`, `unknown-key`, `stale-session`, `replay`,
`not-allowed`, `bad-arg`, `busy`, `too-big`, `quota`, `no-card`, `rate`.

### 5.5 The allow-list

A table of its own (`opendmd_core::connect::proto::ALLOWED`), not the HTTP
router. Anything not in it is `not-allowed`, and the code has no command
for anything else. At compile time no name in it may contain any of
`update`, `firmware`, `ota`, `www`, `delete`, `format`, `rename`, `wifi`,
`ssid`, `hostname`, `key-`, `api`, `token`, `replay`, `reboot`, `restart`,
`reset`, `timing`, `network`, `config`, `file`, `read`, `upload`.

| `c` | Arguments | Rules | Becomes |
|---|---|---|---|
| `play` | `path` | a GIF under `/gifs/` ending `.gif`, at most 240 bytes, no empty, `.`-leading, `.`- or space-ending or `..` segment (FAT drops a trailing dot or space, so two spellings would name one file), no `\ ? # : * < > \| "` or control character; or `@formula:<stem>` or `/formulas/<stem>.psi` with `<stem>` of `[a-z0-9_-]`, 1 to 64, starting with a letter or digit. Never `/cache/`, an inline Formula, `~sent` or a virtual file | `PlayNow`, with the LAN's own play validation too |
| `next`, `prev`, `pause` | none | | `Skip`, `Back`, `TogglePause` (`r`: `{paused}`) |
| `brightness` | `v` | an integer, 0 to the panel's `max_brightness` | `ApplyConfig` |
| `playlist` | `set` | `""` (the main list) or one name of at most 64 bytes: no `/`, `\`, `..`, control character, leading `.` or trailing `.` or space | `ApplyConfig`; `ack` may say `pending` |
| `playlists` | none | | `r`: the `/sets` listing, bounded to fit the panel's 2,048-byte text limit |
| `formulas` | `after` (optional) | a stem | `r`: the Formulas this panel can play by reference, built in and on its card, a page at a time within the panel's 2,048-byte text limit (5.6) |
| `formula-add` | `id, stem, name, src_size, src_sha256, psb_size, psb_sha256` | 5.7 | an add, its bytes to follow |
| `formula-part` | `id, off, data` | 5.7 | the add's bytes; the last one lands it |
| `send` | `id, name, size, sha256, copy_size, copy_sha256, play` | section 8 | a transfer |
| `send-cancel` | `id` | | |
| `controllers` | none | | `r`: `[{k, label, f, added, added_by, last_s}]` |
| `add-controller` | `pub`, `label` | only from a key with `can_add` (else `not-allowed`); a usable key not on the list; a label that survives cleaning; `quota` when the list has 8 | stored with no flags; `keys` follows |
| `remove-controller` | `k` | a key on the list; any paired key may remove any key, itself included | removed; `keys` follows; the server closes its sockets |
| `name` | `name` | at most 32 characters after cleaning | stored; `info` follows |

### 5.6 `formulas`

What a controller can offer to `play`, as the panel itself knows it (a
panel up to 1.1.0-beta.4 answers `not-allowed`: a controller then offers
the Formulas built into that firmware from its own copy of the list).
Arguments: none for a listing's first page; `{"after":"<stem>"}` for the
page after that stem (a stem by the rule below, else `bad-arg`). Any
other member of `a` is `bad-arg`, so a later controller can tell a panel
that lacks a later argument.

```json
{"abi":3,"psb":true,"builtin":[["fire","Fire"],["big-clock","Big clock"]],"card":[["my-road","My road"]],"more":false}
{"abi":3,"psb":true,"builtin":[],"card":[["saved-023","Saved"],["saved-024","Saved"]],"more":true,"next":"saved-024"}
```

- `abi` and `psb` (a panel that takes `formula-add`, 5.7): the compiled
  format it plays, and whether it wants the controller's compiled file
  with the source (true on every board today; a board able to compile
  while Connect is open would say false and compile the source itself).

- `builtin`: the Formulas built into the firmware, `[stem, name]` each,
  played as `@formula:<stem>`; `name` from its `name "…"` line, else
  the stem's title (`big-clock` → "Big clock"). On the first page only
  (empty on the pages after it); empty in a build without Formulas.
- `card`: the card's, played as `/formulas/<stem>.psi`, **in stem order**
  (byte order), those after `after`: each `<stem>.psi` in `/formulas/`
  whose stem is one a remote `play` may name (5.5: `[a-z0-9_-]`, 1 to 64,
  a letter or digit first) and whose compiled `<stem>.psb` (or, for one
  added by remote and not yet compiled here, its `<stem>.psr`) beside it is
  ready for this firmware (one that is missing, empty, marked as not
  compiling, or compiled for another version is left out: it would not
  play). `name` is the one the source's `name "…"` line gave the compiled
  file, else the stem's title. `null` when the panel has no card it can
  read (and then `more` is false); `[]` when there are none (left).
- **Bounded, and paged.** The result is at most 1,948 bytes (2,048 less
  100 for the `ack` around it), whatever its end turns out to be. Built-in
  ones first, then the card's in order; the first entry that would not
  fit, and every one after it, is left out (never a later, shorter one in
  its place), and `more` is `true`. `more` is also `true` when the panel
  stopped early: it opens no more than 48 compiled files a page, and stops
  at once (never waiting) while a file transfer is under way on the card.
  With `more`, `next` (when present) is the `after` for the next page: the
  last stem the panel looked at. `more` without `next`: ask the same page
  again later. `more: false`: the list is complete.
- **Its own rate, per panel.** A listing's first page (no `after`, or an
  `after` other than the last `next` the panel gave) at most one every
  5 s, else `rate`. The pages that follow it, each with the `after` the
  previous page gave as `next`, within 60 s of it, are free, up to 8 pages
  in a listing. A controller asks a listing's pages in turn and stops
  after 8.
- The card is read the gentle way: one directory pass noting the stems
  that have a compiled file larger than a mark, a second noting the
  smallest candidate stems after `after` among those (a source with no
  compiled file costs no lookup), and one read of at most 261 bytes of the
  header of each one looked at; nothing is written, and the panel keeps
  playing. No list is kept between requests. **Its worst case**, a
  listing's 8 pages within the 5 s rate: 8 × (two directory passes + at
  most 48 header reads), i.e. 16 passes and 384 opens; measured on the
  emulator with 256 files in `/formulas/`, a page took at most 218 ms of
  card time (its longest single open 8 ms), with no late frame.

### 5.7 Adding a Formula: `formula-add`, `formula-part`

A controller adds a Formula to the panel's card (the gallery's "Add to
panel"): its source and the compiled file the controller made of it for
the panel's `abi` (5.6), as `/formulas/<stem>.psi` and `<stem>.psb`.

**Not in releases yet.** The two commands are built only with the
firmware's `connect-formula-add` feature (test images); every release and
beta answers them `not-allowed`, and its `formulas` reports no `abi` or
`psb`, so a controller offers no Add to panel (a release image's check,
`tools/release-test-features-check.sh`, holds the code out of it).

1. `formula-add {id, stem, name, src_size, src_sha256, psb_size,
   psb_sha256}`: `id` a non-zero 32-bit number; `stem` the web app's save
   rule, `[a-z0-9-]`, 1 to 40, a letter or digit first; `name` what the
   source's `name` line says (at most 60 characters after cleaning);
   `src_size` 1 to 16,384 and `psb_size` 6 to 8,192 (more: `too-big`);
   both hashes hex. The panel looks for where it lands: the stem, else
   `<stem>-2` … `<stem>-9` (a stem is taken when its `.psi`, `.psb` or
   `.psr` is there, as one directory pass sees it). **A taken stem whose
   source has the signed length and `src_sha256` is the same Formula: the
   answer is `r {done: true, stem}` and nothing is written.** None free:
   `quota`. Otherwise `r {next: 0}`. Before the card is read at all: the
   add's own rate (one `formula-add` in 2 s per panel: `rate`), the same
   add signed again (a reconnect: `r {next}` from where it was, or `done`
   again if it landed), another key's add under way (`busy`, until 10
   minutes after its last progress; a landed add holds nothing), the day's
   tally (`quota`: 16 adds or 256 KB a day per panel, kept on the card in
   `/formulas/.add-day` so a restart does not reset it; the day is the
   panel's clock's, and with no clock the tally stands until a clock shows
   a new day; a tally it cannot read counts as a full day). In the pass:
   at most 128 Formulas on the card (`quota`). Also refused: `no-card`,
   `busy` (a LAN transfer or update on the card), `bad-arg`. The same
   key's other add replaces its stalled one; a key removed, remotely or on
   the LAN, takes its add with it; a stale add's part files go when it
   goes stale.
2. `formula-part {id, off, data}`: `data` base64url (no padding, canonical)
   of 1 to 384 bytes (512 characters: the largest such body stays within
   700 bytes), strictly at the panel's `next`; the stream is the source then
   the compiled file. Each part's `ack` is `r {next}`. A part out of order
   (a part sent again, a replay past the counter) is `bad-arg` and nothing
   is written; another key's is `not-allowed`.
3. On the last part the panel checks both SHA-256s, that the source is
   UTF-8 with no NUL, that the compiled file is `PSB` with this player's
   ABI and that every program in it loads and passes `psb::verify` (behind
   a heap gate: the compiled bytes, one program and 20 KB for the rest;
   below it `busy`, the bytes kept for the same `formula-add` signed
   again); then renames the program to **`<stem>.psr`** and then the
   source to `<stem>.psi` (a rename never replaces a file), and answers
   `r {done: true, stem}` (the stem it landed under). A check that fails
   is `bad-arg`, and nothing is kept; a second rename that fails takes the
   first back; a restart between the two leaves a `.psr` with no source,
   which the next start removes. Parts and the landing wait while a LAN
   transfer or update is under way.

**The one exception to "no program from a remote controller".** Every other
path keeps a remote controller's programs off the panel: `play` takes no
inline Formula (`@formula?psb=…`) and never `~sent`, so a compromised page
or a hostile relay can choose among programs the panel already has, never
supply one. `formula-add` is the exception, and what bounds it is:

- **checked, then never on the JIT unless the panel's own compiler made
  it, by construction:** the program is loaded and verified like any
  program from the network (`psb::verify`) and stored as `<stem>.psr`,
  never `.psb`. The player reads a card Formula's program in one place
  (`fx_psi::card_program`): the `.psb` if it opens, else the `.psr`, and
  what it read from a `.psr` is held to the bounds-checked VM (the JIT's
  build path asks `fx_psi::jit_refusal` before anything else). Only the
  panel's own compiler writes `<stem>.psb`, from the source; once it has,
  and its own output read back and checked, the `.psr` is removed. There
  is no record to keep, load, race or lose: a missing, torn or unreadable
  anything is "on the VM" or "not playable", never "JIT". A Formula the
  board can never compile keeps its `.psr` and stays on the VM for good.
  A source that does not compile there is marked as not compiling, and
  its mark (a `.psb`) wins over the `.psr`: it never plays again;
- **create-new only:** two files under `/formulas/`, their names from the
  stem rule (no separator, no dot), renamed into place where nothing is;
  nothing is overwritten or deleted, and a different source under a taken
  stem gets the next free stem;
- **sizes and counts:** 16 KB of source, 8 KB compiled, 384 bytes a part,
  16 adds and 256 KB a day (kept across restarts), 128 Formulas on the
  card, one add at a time, one `formula-add` in 2 s;
- **signed bytes:** every part is a signed command under the session's
  counter, and both hashes are in the signed `formula-add`: a relay can
  drop or delay parts, never alter or replay one.

Inline and `~sent` play stay excluded: they would hand the panel a
program with none of the above (no record, no recompile, nothing on the
card to check later) for every linked panel at once.

On the VM a heavy Formula costs about a third more than on the JIT (the
gallery's largest, rift-patrol: ~9.3 against ~7.0 ms a frame by the cost
model; 4.7 against 3.4 ms in the emulator), well inside a frame. A Formula
whose compile needs more heap than the panel ever has stays unconfirmed,
on the VM, for good: on the classic ESP32 that is the gallery's largest
(rift-patrol's compile needs ~147 KB; the classic has ~93 KB at its mount
pass), while small ones are compiled at the next start and then may use
the JIT.

## 6. Pairing

### 6.1 From the panel (LAN link, QR)

The panel makes a pairing secret `S`: 100 random bits as 20 base32
characters. Its ASCII text is the HMAC key below. One window per secret:
**5 minutes, one use, three tries**.

1. On the LAN, `POST /api/connect/pair` (the API key applies). The panel
   opens the window and sends `pair-open {pair_id, ttl}`, with
   `pair_id` = the first 10 bytes of SHA-256(`"opendmd pair id v1"` ‖ `S`).
   The server sees `pair_id`, never `S`. The route answers the link
   `https://opendmd.dev/c#<S>` (`/c` hands the fragment to `/connect/`),
   which the web app opens; with `?qr=1` the panel also shows it as a QR
   code (42 bytes, version 3-L,
   byte mode, the mask the standard's penalty rules pick:
   `opendmd_core::connect::qr`), one LED a module: light modules lit, dark
   ones off, on a lit quiet zone of one row above, two below and four
   columns either side (`connect::screen`). The LAN link never depends on
   the QR code.
2. The controller posts `pair_id` to `/v1/find` and gets `dev`.
3. It makes its key pair, joins as a `guest` with `pair_id`, and sends
   `pair {pub, label, mac}` with
   `mac` = HMAC-SHA256(`S`, `"opendmd pair v1"` ‖ `dev` ‖ `pub` ‖ label)
   (`dev` and `pub` as raw bytes; the label last, as its UTF-8 bytes exactly
   as sent after JSON unescaping: every other field has a fixed length). A
   server that rewrites the label breaks the MAC. The controller cleans
   and bounds the label itself (section 3's rule, well-formed UTF-16 in a
   browser) before it computes the MAC: a correct MAC uses the window up
   before the panel cleans the label, and a label that does not survive
   cleaning is then refused `bad-arg`, window spent.
4. The panel checks the MAC in constant time, over the label as received,
   and cleans the label (section 3) only after that. A wrong one spends a try
   (`paired {err:"bad-mac"}`); after the third, or after 5 minutes, the
   window is closed (`closed`, `expired`). On success it stores the key
   (flag `can_add`), shows "Remote control added" for 10 s, and only after
   the NVS write returned sends `keys` (the new list, so the server knows
   the key before the new controller attaches; if the panel's budget for
   `keys` is spent, `keys` follows within the second instead) and answers
   `paired {g, k, mac2}` with `mac2` = HMAC-SHA256(`S`,
   `"opendmd paired v1"` ‖ `dev` ‖ `pub`), then `pair-close`. The panel sends `pair-close` the moment any window
   dies: used, expired, out of tries, cancelled on the LAN, replaced by a
   new one, Remote control switched off; and, after a reconnect, for a
   window it announced that died meanwhile. A store that fails answers
   `paired {err:"busy"}` (the window is used up: a new link is needed).
5. The controller checks `mac2`. A server that swaps in a key of its own
   can make neither MAC.

### 6.2 Add another device (from a paired controller)

1. The inviter makes a secret of 128 random bits (26 base32 characters),
   shows `https://opendmd.dev/c#add=<dev>.<secret>` as a link and a QR on
   its own screen, and sends `invite-open {invite_id, ttl}` with
   `invite_id` = the first 10 bytes of SHA-256(`"opendmd invite id v1"` ‖
   secret).
2. The new browser joins as a guest with `invite_id` and sends
   `add-req {pub, label, mac}`, `mac` = HMAC-SHA256(secret,
   `"opendmd add v1"` ‖ `dev` ‖ `pub` ‖ label), the label as in 6.1. The
   inviter checks it, then cleans the label.
3. Both screens show the same 4 characters: the first 20 bits of
   SHA-256(`"opendmd confirm v1"` ‖ `pub`) in base32, upper case. The user
   confirms on the inviter, which signs `add-controller {pub, label}`.
   That stops anyone who saw the link from adding themselves first. It does
   not stop a link thief working with the server: the code is 20 bits, so
   they could grind a key whose code matches (about 2^20 tries) and have
   the server deliver it in place of the real guest's. Accepted for P1:
   the link is shown only on a paired screen, and the panel lists every
   key with who added it.
4. On its `ack`, the inviter sends `add-ok {g, k}` to the guest. The new
   key has no `can_add`.

## 7. Status

Sent by the panel only while watched: on change, at most one every 2 s on
the classic, and one every 30 s. The events snapshot through a field mask,
about 250 to 400 bytes: `now_playing {path, title, elapsed_ms, next,
kept_s, seed, sent}`, `paused`, `brightness`, `max_brightness`, `set`,
`context.state`, `transfer`, `fw`, `name`, `id`. Never heap, card or
network internals. The server passes it to every attached controller and
keeps none of it. Like an `ack`, status is not authenticated: a hostile
server can show a controller a false state, never change the panel's.

## 8. Sending a GIF

- Signed `send {id, name, size, sha256, copy_size, copy_sha256, play}`:
  `id` a non-zero 32-bit transfer id; `name` a file name only (no `/`, no
  control character, not `.`-leading, ending `.gif`, at most 120
  characters, at most 229 bytes, so `/gifs/Sent/<name>` fits a remote play's 240) for `/gifs/Sent/`, never overwriting (a taken name gets the
  web app's next name: `x_09.gif` → `x_10.gif`, `cat.gif` → `cat_2.gif`);
  `size` 1 to 524,288 (more is `too-big`); `sha256` hex; `copy_size` 0
  (no prepared copy) to 524,288, with `copy_sha256` exactly when it is
  not 0; `play` a boolean (default false). A panel whose `hello` said
  `caps.send: false` answers `not-allowed`.
- Its `ack` comes once the panel has opened the file: ok with `r {offset}`,
  always (0 for a new transfer, else where a resume carries on from; a
  resume also says `resumed: true`), then the first `credit`. Refusals:
  `busy` (a LAN upload or an update under way: the LAN always wins; or
  another key's transfer held), `no-card`, `quota`, `bad-arg` (the held
  transfer's id with other arguments).
- The bytes follow as one stream, the GIF's then the copy's (the copy as
  the web app's `prepareGif` makes it; none past 256 colours), in `0x02`
  chunks of at most 1,024 bytes, strictly in order, and only inside the
  panel's last `credit {id, upto, k}`. The panel grants more only after the
  card write returned, and never more than `caps.win` chunks past what it
  has written (2 on the classic, 4 on the S3): the server forwards a
  chunk only inside the last credit and at the next offset, the first
  after a `send` anywhere from `upto` less `caps.win` chunks (a resume). A
  chunk at another offset or past `upto` abandons the transfer. The
  server routes credits and chunks by (key, id) and holds nothing.
- The panel writes `<name>.<id>.part` (and `<name>.<id>.copy.part`) in
  `/gifs/Sent/`, checks both SHA-256s at the end and that the GIF is a GIF
  and the copy a prepared copy (a GIF89a with its `OPENDMD1LZW` hint at a
  cap of at most 9, `docs/cache-format.md`), renames the GIF into place,
  moves the copy to its place under `/cache/`, and plays the GIF once if
  `play` (then the playlist carries on). It lands in no playlist.
- **The last `credit`** of a transfer says how it ended. Landed (after the
  last card write and both hash checks): `{id, upto: <total>, k, done:
  true, name}`, `name` the file name it was saved under in `/gifs/Sent/`
  (after any `_2`). Failed: `{id, upto, k, err}`, `upto` (always an integer) where it was and
  `err` a code: `bad-arg` (the bytes miss the signed hashes, are not a GIF
  or the copy not a prepared copy, or a chunk came outside the credit),
  `busy` (the card failed), `not-allowed` (the starting key was removed).
  Either way the `.part` files are gone.
- Resume: the panel keeps `{id, offset, hash state}` in RAM for 10 minutes
  after the last progress; the same `send` signed again by **the key that
  started it** (after a reconnect) gets `ack r {offset, resumed: true}` and
  `credit` from the offset, nothing charged again; a controller uses only
  the credits that come after that ack. Another key's `send` while it is
  held is `busy`; the same key's `send` for another transfer replaces its
  stalled one (abandoned, no refund). The same `send` signed again after
  its transfer **landed** (within the 10 minutes, from its key: a lost
  `done`) answers ack ok `r {offset: <total>}` and its `done` credit again:
  never a second file, nothing charged. `send-cancel {id}` from its key
  abandons it, in any later session too (another key's: `not-allowed`; no
  such transfer: ok). `.part` files are removed at boot.
- `hello`'s `caps {send, win}` are the panel's own: `send` true only when
  this panel takes sends: on the ESP32-S3; **off on the classic ESP32**
  until device test D4 shows its card writes sit beside play (a build
  with `connect-send-classic` turns them on there, `connect-no-send` off
  anywhere); `win` the window above. The server passes them to
  controllers.
- Quotas: 8 MB per rolling 24 h on the panel (`size + copy_size`, at
  least 64 KB, charged when a send is accepted, never on a resume, never
  refunded), 16 MB and
  60 sends a day per panel at the server (`quota`).

## 9. Limits

| Limit | Value |
|---|---|
| Text frame | 1,024 bytes to the panel and from controllers; 2,048 from the panel |
| Command body | 700 bytes |
| Chunk data | 1,024 bytes |
| One file in a send | 524,288 bytes |
| Controllers per panel | 8 |
| Label | 24 bytes |
| Panel name | 32 characters |
| Remote play path | 240 bytes |
| `n` | 2^53 - 1 |
| Rate | 4 commands a second per controller, 8 per panel |
| Pairing window | 5 minutes, one use, 3 tries |
| Guests | 2 at a time, 60 s each |
| Guest slot `g` | assigned 1 to 255; read 0 to 2^31 - 1 (to 255 up to 1.1.0-beta.4) |
| A Formula added by remote | 16 KB of source, 8 KB compiled, 384 B a part; 16 adds and 256 KB a day; 128 Formulas on the card; one add in 2 s |
| `formulas` | a listing's first page one in 5 s per panel; 8 pages a listing |

## 10. Test vectors and mocks

`opendmd-core/tests/connect-vectors.json` holds throwaway keys and ids
made from fixed strings (never use one for anything real) and, for each,
what every implementation must compute: the id and its tag, the register
body, the key blob and `keys` message, attach signatures, pairing and
invite MACs (with a label that needs escaping, and one rewritten), the confirm code, version negotiation, acks, and 88 command
frames, each with what the panel answers (`expect`) and what the server's
pre-check does (`server`). The cases cover the red list's single-frame
attacks: replays, unsigned, tampered and re-serialised frames, wrong
signer, wrong session or panel, every never-remote name, duplicate keys,
bad paths and arguments, oversize and malformed bodies.

- The Rust core makes the file and checks itself against it
  (`opendmd-core/tests/connect_vectors.rs`; `CONNECT_VECTORS=write` after a
  deliberate change, then review the diff). The stateful red cases (rates,
  reconnects, removal, pairing windows, transfers) are in
  `opendmd-core/tests/connect_red.rs`.
- `tools/connect/vectors_check.py` checks the Python implementation
  against it (part of `make test`).
- `make connect-mock` runs the mock server, the mock panel and controllers
  on localhost: registration, pairing, commands, add another device,
  removal, reconnects, and the red list over the wire.
- The Worker and the web controller check themselves against the same
  file.
