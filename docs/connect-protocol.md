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
| panel → server → one controller | `credit` | `id`, `upto` |
| panel → server → one guest | `paired` | `g`, then `k` and `mac2`, or `err` |
| panel → server | `forget` | the last message under an identity being dropped: the server deletes its record |
| server → panel | `watch` | `on`: whether any controller is attached |
| server → panel | `pair` | a guest's `pair`, with `g` (the guest's slot) added |
| server → panel | binary `0x01`, `0x02` | relayed from controllers |

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
result, or drop a real one; it still cannot make the panel do anything.
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
| `send` | `id, name, size, sha256, copy_size, copy_sha256, play` | section 8 | a transfer |
| `send-cancel` | `id` | | |
| `controllers` | none | | `r`: `[{k, label, f, added, added_by, last_s}]` |
| `add-controller` | `pub`, `label` | only from a key with `can_add` (else `not-allowed`); a usable key not on the list; a label that survives cleaning; `quota` when the list has 8 | stored with no flags; `keys` follows |
| `remove-controller` | `k` | a key on the list; any paired key may remove any key, itself included | removed; `keys` follows; the server closes its sockets |
| `name` | `name` | at most 32 characters after cleaning | stored; `info` follows |

## 6. Pairing

### 6.1 From the panel (LAN link, QR)

The panel makes a pairing secret `S`: 100 random bits as 20 base32
characters. Its ASCII text is the HMAC key below. One window per secret:
**5 minutes, one use, three tries**.

1. On the LAN, `POST /api/connect/pair` (the API key applies). The panel
   opens the window and sends `pair-open {pair_id, ttl}`, with
   `pair_id` = the first 10 bytes of SHA-256(`"opendmd pair id v1"` ‖ `S`).
   The server sees `pair_id`, never `S`. The web app opens
   `https://opendmd.dev/connect/#pair=<S>`; with `?qr=1` the panel also
   shows a QR of `https://opendmd.dev/c#<S>` (42 bytes, version 3-L,
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
   `"opendmd paired v1"` ‖ `dev` ‖ `pub` ‖ label), the label's bytes as
   received (so a label rewritten on the way in shows on the way back),
   then `pair-close`. The panel sends `pair-close` the moment any window
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
  characters) for `/gifs/Sent/`, never overwriting (a taken name gets a
  suffix); `size` 1 to 524,288 (more is `too-big`); `sha256` hex;
  `copy_size` 0 (no prepared copy) to 524,288, with `copy_sha256` exactly
  when it is not 0; `play` a boolean (default false).
- The bytes follow as one stream, the GIF's then the copy's, in `0x02`
  chunks of at most 1,024 bytes, strictly in order, and only inside the
  panel's last `credit {id, upto}`. The panel grants more only after the
  card write returned (2 chunks at a time on the classic, 4 or more on the
  S3: `caps.win`). A chunk at another offset or past `upto` abandons the
  transfer. The server forwards chunks only for a transfer its socket
  started and holds nothing.
- The panel writes `<name>.part`, checks both SHA-256s at the end, renames,
  and plays it once if `play`. A wrong hash removes the `.part`.
- Resume: the panel keeps `{id, offset, hash state}` in RAM for 10 minutes;
  the same `send` signed again after a reconnect gets `credit` from the
  offset. `.part` files are removed at boot.
- Quotas: 8 MB per rolling 24 h on the panel, 16 MB and 60 sends a day per
  panel at the server (`quota`). No card: `no-card`. A LAN upload always
  wins: a remote send during one is `busy`.

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

## 10. Test vectors and mocks

`opendmd-core/tests/connect-vectors.json` holds throwaway keys and ids
made from fixed strings (never use one for anything real) and, for each,
what every implementation must compute: the id and its tag, the register
body, the key blob and `keys` message, attach signatures, pairing and
invite MACs (with a label that needs escaping, and one rewritten), the confirm code, version negotiation, acks, and 81 command
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
