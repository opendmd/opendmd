# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""A fake OpenDMD ESP32 device, for testing the browser management UI without
hardware.

    uv run tools/mock-device.py --root ~/scratch/dmdos-card [--overlay DIR] \
        [--www opendmd-web/www] [--host 127.0.0.1] [--port 8080] [--dwell SECS] \
        [--strict] [--quiet]

    uv run tools/mock-device.py --proxy http://192.168.1.69 [--www ...] [--port 8080]

Two modes.

**Card mode** (`--root`) fakes the whole device: the JSON API in
`firmware/src/http.rs`, the card file server in `firmware/src/files.rs`, and a
simulated now-playing loop, all served on top of a local copy of an SD card
(`--root`, an extracted card: `gifs/`, `odm/` (prepared copies),
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
  `main.rs`.
- **No `GET`/`DELETE /api/sets/*` route** — sets are read at
  `GET /sets/<name>.txt` and deleted at `DELETE /api/files/sets/<name>.txt`.
- **`.gz` preference is `www/`-only**, and unconditional (no `Accept-Encoding`
  check) — the content type still comes from the logical, un-gzipped name.
- **Directory listings are unsorted** (`os.scandir` order, standing in for
  FAT order) and sizes are opt-in (`?sizes=1`), matching the firmware's
  quadratic-`stat`-avoidance note in `files.rs`.
- `GET /api/config` never echoes `wifi_pass`, only `wifi_pass_set`.
- **The default manifest** is `filelist.txt`: `set` = `""` plays it,
  `"filelist.*"` selects that root manifest, anything else `sets/<set>.txt`.
  An obsolete `filelist.odm.txt` is ignored and refused as a set. A set whose
  manifest does not exist is a **404 `{"error":"set not found"}` with nothing
  saved**; a `/`, `\\` or `..` in it is a 400. `manifest` in status, the 409 on
  deleting the active manifest and the "is this install active" check all use
  the resolved path.
- **The playback cache.** `GET /api/cache` is `{total, prepared, state,
  count_ms}`: `total` the playlist length, `prepared` how many of its entries
  have a prepared copy at their bucketed path (`odm/<Category>/<bb>/X.odm`),
  counted in a background thread after any change (an upload or delete under
  `odm/`, a forget, a GIF delete, a playlist rebuild) — the first request
  after one answers `"counting"` with the previous figure (null before the
  first count), as the device does. `GET /api/cache/list` is every copy's
  path; `POST /api/cache/forget {"path":"/gifs/…"}` deletes a GIF's copy;
  `DELETE /api/files/gifs/…` takes the copy with it.
- **`/filelist*`** serves every root manifest (`filelist.txt`, `.hall`,
  `.xmas`, an obsolete `.odm.txt`) as `text/plain`, `no-cache`; a `/` after
  the name is a 404.
- **`POST /api/play {"path"}`** puts the file on "the panel" now (an
  off-playlist `now_playing`, ordinal and `gifs_started` bumped), and the
  schedule resumes at the next dwell. 404 if it does not open, 415 if it does
  not start with `GIF8`/`ODM1` (the firmware's error text there comes from the
  decoder; here it is a fixed one), 400 for a relative or escaping path.
- **`GET /api/stat?path=`** is `{path, exists, size, is_dir}`, `size` null for
  a directory or a miss, `/` a directory.
- **`/api/status`** has `index_state` (`ready`, or `unavailable` when the
  manifest does not open; `building` is never observable, as on the device,
  because a rebuild holds the lock a status read needs) and `www_version`
  (the first line of `--www`/version.txt, trimmed, <= 64 chars, else null).
- **JSON bodies over 8 KB** (`PUT /api/config`, `POST /api/play`) are a 413
  `{"error":"body is N B, the limit is 8192 B"}`, judged on `Content-Length`.

# `--strict`

Behave like the current firmware where this mock otherwise differs on
purpose: a `..` escape through the `/*` catch-all is a plain 404 (not 400),
and `/api/stat` and `/api/play` cost a FAT directory scan of the parent
(15 ms + 0.15 ms per entry: ~20 ms in a bucket, ~590 ms in `gifs/Consoles`),
so a UI that stats or plays in a big directory feels it.
- OTA validates the ESP image magic byte (`0xE9`) and the 1,572,864 B slot
  size, and demands a `Content-Length`.

# Deliberate deviations from the firmware (per this tool's own spec)

- The real firmware's top-level `/*` catch-all does **not** 400 on a `..`
  escape: `files::card_path` returns `None`, the handler's `if let Some(..)`
  is simply skipped, and it falls through to a plain 404. Every other route
  (`/www/*`, `/gifs/*`, `/odm/*`, `/sets/*`, `/api/files/*`, `/api/files`) does
  send 400. Without `--strict` the catch-all 400s too.
- serde's exact "wrong type" error text is not reproduced verbatim (there is
  no serde here); a similarly shaped `"invalid JSON: ..."` message naming the
  offending field is produced instead.

# The now-playing simulation

Status carries `t_ms` (ms since the mock "booted") and, in `now_playing`,
`elapsed_ms` and `frame`: the file's position runs on a simulated clock from
the moment it started (paused: frozen; resumed: shifted by the pause, as the
engine does) through the file's real frame delays — the prepared copy's ODM
frame headers when there is one, else the GIF's Graphic Control Extensions
with the player's rules (0 -> 100 ms, else >= 20 ms), identical to the ODM
delays for all 11,039 copies on the card. An ideal device: never late.

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
import http.client
import http.server
import json
import math
import os
import random
import re
import shutil
import sys
import threading
import time
import urllib.parse
from pathlib import Path

# --------------------------------------------------------------------------
# Constants shared with the firmware (config.rs, files.rs, ota.rs)
# --------------------------------------------------------------------------

MANIFEST_REL = "filelist.txt"          # config::MANIFEST_PATH, relative to the card root
OBSOLETE_SET = "filelist.odm.txt"      # config::OBSOLETE_ODM_MANIFEST: ignored, refused as a set
MAX_JSON_BODY = 8192                   # http.rs MAX_JSON_BODY
SET_NOT_FOUND = "set not found"        # state::SET_NOT_FOUND
SETS_DIR = "sets"                      # config::SETS_DIR
OTA_SLOT_SIZE = 1_572_864              # the board's OTA slot (ota.rs)
SD_CAPACITY_BYTES = 31_266_439_168     # a plausible "measured" capacity for /api/status

DEFAULT_SETTINGS = {
    "wifi_ssid": "MockNet",
    "wifi_pass": "secret",
    "brightness": 96,
    "dwell_min": 3,
    "dwell_max": 30,
    "shuffle": True,
    "platforms": "",
    "set": "",
    "hostname": "opendmd-mock",
}

# field -> expected type, for PUT /api/config validation
FIELD_TYPES = {
    "wifi_ssid": "str",
    "wifi_pass": "str",
    "brightness": "u8",
    "dwell_min": "u64",
    "dwell_max": "u64",
    "shuffle": "bool",
    "platforms": "str",
    "set": "str",
    "hostname": "str",
}

STATUS_TEXT = {
    200: "OK",
    302: "Found",
    400: "Bad Request",
    404: "Not Found",
    405: "Method Not Allowed",
    409: "Conflict",
    413: "Payload Too Large",
    415: "Unsupported Media Type",
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
        "odm": "application/octet-stream",
        "jpg": "image/jpeg",
        "jpeg": "image/jpeg",
        "svg": "image/svg+xml",
        "ico": "image/x-icon",
        "txt": "text/plain; charset=utf-8",
    }.get(ext, "application/octet-stream")


def cache_for(rel: str) -> str:
    if is_root_manifest_name(rel):
        return "no-cache"
    if "/pkg/" in f"/{rel}":
        return "max-age=3600"
    ext = Path(rel).suffix.lstrip(".").lower()
    if ext in ("html", "htm", "js", "mjs", "json"):
        return "no-cache"
    return "max-age=3600"


def is_manifest_path(rel: str) -> bool:
    """Port of files::is_manifest_path, in root-relative terms."""
    if rel.endswith(".idx") or rel.endswith(".tmp") or rel.endswith(".part"):
        return False
    parts = rel.split("/")
    if len(parts) == 1 and parts[0].startswith("filelist."):
        return True
    if len(parts) == 2 and parts[0] == SETS_DIR:
        return True
    return False


PLATFORM_PREFIX_RE = re.compile(r"^[A-Za-z0-9]+_")
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


def www_version(www_root: Path):
    """files::read_www_version: the first line of version.txt, trimmed,
    at most 64 characters; None if missing or blank."""
    try:
        with open(www_root / "version.txt", "rb") as f:
            text = f.read(128).decode("utf-8", "replace")
    except OSError:
        return None
    line = (text.splitlines() or [""])[0].strip()
    return line[:64] or None


def fnv1a32(b: bytes) -> int:
    h = 0x811C9DC5
    for x in b:
        h = ((h ^ x) * 0x01000193) & 0xFFFFFFFF
    return h


def bucketed_path(path: str):
    """opendmd_core::layout::bucketed_path: /gifs/Cat/X.gif ->
    /odm/Cat/<bb>/X.odm, None for anything else."""
    parts = path.lstrip("/").split("/", 1)
    if len(parts) != 2 or parts[0].lower() != "gifs" or not parts[1].lower().endswith(".gif"):
        return None
    rest = parts[1]
    d, _, name = rest.rpartition("/")
    if len(name) <= 4:
        return None
    odm = name[:-4] + ".odm"
    bb = f"{fnv1a32(odm.encode()) & 0xFF:02x}"
    return f"/odm/{d}/{bb}/{odm}" if d else f"/odm/{bb}/{odm}"


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


def odm_delays(b: bytes) -> list[int]:
    """The `delay_ms` of every ODM frame header (docs/odm-format.md)."""
    if b[:4] != b"ODM1" or len(b) < 32:
        return []
    count = int.from_bytes(b[9:13], "little")
    i = 32 + 3 * int.from_bytes(b[13:15], "little")
    out = []
    for _ in range(count):
        if i + 6 > len(b):
            break
        out.append(int.from_bytes(b[i:i + 2], "little"))
        i += 6 + int.from_bytes(b[i + 2:i + 6], "little")
    return out


def enoent(path: str) -> str:
    """Rust's io::Error Display for ENOENT, which the firmware's error
    messages embed verbatim."""
    return f"{path}: No such file or directory (os error 2)"


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
            return len(data)

    def odm_files(self) -> list[str]:
        """Card paths of every `.odm` under odm/ in either layer, tombstones
        dropped: the prepared copies."""
        with self.lock:
            found: dict[str, str] = {}
            for base in (self.root_dir, self.overlay_dir):
                if base is None or not (base / "odm").is_dir():
                    continue
                for d, _, names in os.walk(base / "odm"):
                    for n in names:
                        if not n.lower().endswith(".odm"):
                            continue
                        rel = (Path(d) / n).relative_to(base).as_posix()
                        if rel not in self.tombstones:
                            found[rel.lower()] = "/" + rel
            return list(found.values())

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
        self.paused = False
        self.playlist: list[str] = []
        self.order: list[int] = []
        self.pos = -1
        self.last_advance = 0.0
        self.deadline_misses = 0
        self.worst_lateness_ms = 0
        self.manifest = MANIFEST_REL
        self.index_state = "building"
        self.played: str | None = None  # an off-playlist POST /api/play
        # The file's own clock: its position is (now or paused_at) - file_t0,
        # advanced through the real frame delays (see position()).
        self.file_t0 = time.monotonic()
        self.paused_at = 0.0
        self.delays: dict[str, list[int]] = {}
        self.rebuild_playlist()

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

    def rebuild_playlist(self):
        rel = self.manifest = self.resolve(self.ctx.settings["set"])
        lines = self.ctx.fs.read_lines(rel)
        self.index_state = "ready" if lines is not None else "unavailable"
        filtered = []
        if lines is not None:
            platforms = [
                p.strip().upper()
                for p in self.ctx.settings["platforms"].split(",")
                if p.strip()
            ]
            for raw in lines:
                line = raw.strip()
                if not line:
                    continue
                if platforms:
                    name = line.rsplit("/", 1)[-1].upper()
                    if not any(name.startswith(p + "_") for p in platforms):
                        continue
                filtered.append(line)
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

    def play_now(self, path: str):
        self.played = path
        self.ordinal += 1
        self.gifs_started += 1
        self.last_advance = self.file_t0 = time.monotonic()

    def advance(self):
        self.played = None
        if not self.playlist:
            self.pos = -1
            self.last_advance = time.monotonic()
            return
        self.pos = (self.pos + 1) % len(self.order)
        self.ordinal += 1
        self.gifs_started += 1
        self.last_advance = self.file_t0 = time.monotonic()
        if random.random() < 0.03:
            self.deadline_misses += 1
            self.worst_lateness_ms = max(self.worst_lateness_ms, random.randint(420, 560))

    def set_paused(self, paused: bool):
        """As the engine does: a pause freezes the file's position, and
        resuming shifts its clocks (and the dwell's) by the pause."""
        now = time.monotonic()
        if paused and not self.paused:
            self.paused_at = now
        elif not paused and self.paused:
            self.file_t0 += now - self.paused_at
            self.last_advance += now - self.paused_at
        self.paused = paused

    def frame_delays(self, path: str) -> list[int]:
        """The frame delays of what the device would play for `path`: its
        prepared copy if there is one, else the file. Cached per path."""
        if path not in self.delays:
            out = []
            for cand in (bucketed_path(path), path):
                rel = card_path(cand) if cand else None
                f = self.ctx.fs.read_path(rel) if rel else None
                if f is None:
                    continue
                b = f.read_bytes()
                out = odm_delays(b) if b[:4] == b"ODM1" else gif_delays(b)
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
        at = self.paused_at if self.paused else time.monotonic()
        ms = int((at - self.file_t0) * 1000)
        loops, rem = divmod(ms, loop)
        idx = 0
        while idx < len(delays) and rem >= delays[idx]:
            rem -= delays[idx]
            idx += 1
        return ms, loops * len(delays) + idx + 1

    def tick(self):
        if self.paused or not self.playlist:
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
        self.ordinal = 0
        self.gifs_started = 0
        self.pos = -1
        self.last_advance = 0.0
        self.advance()


def heap_numbers(player: Player):
    t = time.monotonic() - player.boot_time
    free = 57000 + int(1200 * math.sin(t / 4.0)) + random.randint(-150, 150)
    floor = 50000 + int(400 * math.sin(t / 7.0 + 1.0))
    idf_min = 45000
    return max(free, 1000), max(floor, 1000), idf_min


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

    def invalidate(self):
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
            have = {p.lower() for p in self.ctx.fs.odm_files()}
            n = 0
            for p in playlist:
                if p.lower().startswith("/odm/"):
                    n += 1
                    continue
                c = bucketed_path(p)
                if c and c.lower() in have:
                    n += 1
            with self.lock:
                if gen != self.gen:
                    continue
                self.prepared, self.done_gen, self.counting = n, gen, False
                self.count_ms = int((time.monotonic() - t0) * 1000)
                return


class Context:
    def __init__(self, fs: CardFS, www_root: Path, dwell: float, strict: bool):
        self.fs = fs
        self.www_root = www_root
        self.dwell = dwell
        self.strict = strict
        self.settings = dict(DEFAULT_SETTINGS)
        self.lock = threading.RLock()
        self.cache = Cache(self)
        self.player = Player(self)


def ticker_loop(ctx: Context, stop: threading.Event):
    while not stop.wait(0.5):
        with ctx.lock:
            ctx.player.tick()


# --------------------------------------------------------------------------
# HTTP handler (card mode)
# --------------------------------------------------------------------------

def make_handler(ctx: Context, quiet: bool):
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

        def send_json(self, code: int, obj) -> None:
            body = json.dumps(obj, separators=(",", ":")).encode("utf-8")
            self.send_response(code, STATUS_TEXT.get(code, ""))
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            if self.command != "HEAD":
                self.wfile.write(body)
            self._resp_status = code
            self._resp_bytes = len(body)

        def send_error_json(self, code: int, message: str) -> None:
            self.send_json(code, {"error": message})

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

            def m_exact(p):
                return lambda cur: cur == p

            def m_prefix(p):
                return lambda cur: cur.startswith(p)

            routes = [
                (m_exact("/"), {"GET": self.h_root}),
                (m_exact("/api/status"), {"GET": self.h_status}),
                (m_exact("/api/skip"), {"POST": self.h_skip}),
                (m_exact("/api/pause"), {"POST": self.h_pause}),
                (m_exact("/api/config"), {"GET": self.h_config_get, "PUT": self.h_config_put}),
                (m_exact("/api/reindex"), {"POST": self.h_reindex}),
                (m_exact("/api/play"), {"POST": self.h_play}),
                (m_exact("/api/stat"), {"GET": self.h_stat}),
                (m_exact("/api/cache"), {"GET": self.h_cache}),
                (m_exact("/api/cache/list"), {"GET": self.h_cache_list}),
                (m_exact("/api/cache/forget"), {"POST": self.h_cache_forget}),
                (m_exact("/api/files"), {"GET": self.h_files_list}),
                (m_exact("/api/wifi/scan"), {"GET": self.h_wifi_scan}),
                (m_exact("/api/ota"), {"POST": self.h_ota}),
                (m_exact("/api/reboot"), {"POST": self.h_reboot}),
                (m_exact("/api/manifest"), {"PUT": self.h_manifest_put}),
                (m_prefix("/api/sets/"), {"PUT": self.h_sets_put}),
                (m_prefix("/api/files/"), {"PUT": self.h_files_put, "DELETE": self.h_files_delete}),
                (m_prefix("/filelist"), {"GET": self.h_filelist}),
                (m_prefix("/gifs/"), {"GET": self.h_serve_under}),
                (m_prefix("/sets/"), {"GET": self.h_serve_under}),
                (m_prefix("/odm/"), {"GET": self.h_serve_under}),
                (m_prefix("/www/"), {"GET": self.h_www}),
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

        def h_www(self, path):
            rel_raw = path[len("/www/"):]
            rel = card_path(percent_decode(rel_raw))
            if rel is None:
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
            fpath = ctx.fs.read_path(rel)
            if fpath is None:
                self.send_error_json(404, rel_full)
                return
            self.stream_file(fpath, rel, gzipped=False)

        def h_serve_under(self, path):
            rel_full = percent_decode(path)
            rel = card_path(rel_full)
            if rel is None:
                self.send_error_json(400, "path escapes the card")
                return
            fpath = ctx.fs.read_path(rel)
            if fpath is None:
                self.send_error_json(404, rel_full)
                return
            self.stream_file(fpath, rel, gzipped=False)

        # -- JSON API handlers -----------------------------------------------

        def h_status(self, path):
            with ctx.lock:
                player = ctx.player
                player.tick()
                cur = player.current()
                free, floor, idf_min = heap_numbers(player)
                elapsed_ms, frame = player.position()
                obj = {
                    "version": "0.1.0-mock",
                    "variant": "4mb",
                    "uptime_secs": int(time.monotonic() - player.boot_time),
                    "t_ms": int((time.monotonic() - player.boot_time) * 1000),
                    "ip": "127.0.0.1",
                    "wifi": "sta",
                    "hostname": ctx.settings["hostname"],
                    "now_playing": {
                        "ordinal": player.ordinal,
                        "path": cur if cur else "",
                        "title": derive_title(cur, ctx.settings) if cur else "",
                        "elapsed_ms": elapsed_ms,
                        "frame": frame,
                    },
                    "paused": player.paused,
                    "entries": len(player.playlist),
                    "manifest": "/sdcard/" + player.manifest_rel(),
                    "index_state": player.index_state,
                    "www_version": www_version(ctx.www_root),
                    "heap": {"free": free, "floor": floor, "idf_min": idf_min},
                    "refresh_hz": 100,
                    "deadline_misses": player.deadline_misses,
                    "worst_lateness_ms": player.worst_lateness_ms,
                    "gifs_started": player.gifs_started,
                    "sd": {"mounted": True, "capacity_bytes": SD_CAPACITY_BYTES},
                }
            self.send_json(200, obj)

        def h_skip(self, path):
            with ctx.lock:
                ctx.player.advance()
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
                    "dwell_min": s["dwell_min"],
                    "dwell_max": s["dwell_max"],
                    "shuffle": s["shuffle"],
                    "platforms": s["platforms"],
                    "set": s["set"],
                    "hostname": s["hostname"],
                }
            self.send_json(200, obj)

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
                if kind == "u64":
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
                    if field == "hostname":
                        if v != "":
                            next_settings["hostname"] = v
                    elif field in ("dwell_min", "dwell_max"):
                        next_settings[field] = max(1, v)
                    else:
                        next_settings[field] = v

                if next_settings["dwell_max"] < next_settings["dwell_min"]:
                    self.send_error_json(400, "dwell_max must be >= dwell_min")
                    return

                wifi_changed = (
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

                # Settings are "saved" now, like the firmware saves to NVS
                # before the rebuild.
                ctx.settings.clear()
                ctx.settings.update(next_settings)

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
                entries = len(ctx.player.playlist)

            self.send_json(
                200,
                {
                    "ok": True,
                    "entries": entries,
                    "index": index,
                    "index_ms": index_ms,
                    "reboot_required": wifi_changed,
                },
            )

        def h_reindex(self, path):
            with ctx.lock:
                index_ms = random.randint(50, 300)
                time.sleep(index_ms / 1000.0)
                manifest_rel = ctx.player.manifest_rel()
                if ctx.fs.read_lines(manifest_rel) is not None:
                    ctx.fs.write_bytes(manifest_rel + ".idx", b"MOCKIDX1\n")
                ctx.player.rebuild_playlist()
                entries = len(ctx.player.playlist)
            self.send_json(200, {"ok": True, "entries": entries, "index": "built", "index_ms": index_ms})

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
                if magic[:4] not in (b"GIF8", b"ODM1"):
                    self.send_error_json(415, f"{p}: not a GIF or ODM file")
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
            prepared, counting, count_ms = ctx.cache.snapshot()
            with ctx.lock:
                total = len(ctx.player.playlist)
            self.send_json(200, {
                "total": total,
                "prepared": prepared,
                "state": "counting" if counting else "ready",
                "count_ms": count_ms,
            })

        def h_cache_list(self, path):
            paths = ctx.fs.odm_files()
            self.send_json(200, {"paths": paths, "count": len(paths)})

        def forget_copy(self, gif: str) -> bool:
            copy = bucketed_path(gif)
            if copy is None:
                return False
            removed = ctx.fs.read_path(copy.lstrip("/")) is not None and ctx.fs.delete(copy.lstrip("/"))
            if removed:
                ctx.cache.invalidate()
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
            if card_path(p) is None or bucketed_path(p) is None:
                self.send_error_json(400, "path must be a card-absolute /gifs/….gif")
                return
            removed = self.forget_copy(p)
            self.send_json(200, {"ok": True, "path": p, "removed": removed})

        def h_wifi_scan(self, path):
            time.sleep(1.5)
            self.send_json(200, WIFI_SCAN_RESULTS)

        def h_ota(self, path):
            length_hdr = self.headers.get("Content-Length")
            length = int(length_hdr) if length_hdr else 0
            if length == 0:
                self.send_error_json(400, "Content-Length required")
                return
            if length > OTA_SLOT_SIZE:
                # Drain the body so the connection stays usable.
                remaining = length
                while remaining > 0:
                    chunk = self.rfile.read(min(65536, remaining))
                    if not chunk:
                        break
                    remaining -= len(chunk)
                self.send_error_json(413, f"firmware is {length} B, the slot is {OTA_SLOT_SIZE} B")
                return
            data = self.rfile.read(length)
            if not data or data[0] != 0xE9:
                self.send_error_json(400, "firmware validation failed")
                return
            self.send_json(200, {"ok": True, "rebooting": True})

        def h_reboot(self, path):
            with ctx.lock:
                ctx.player.reset_boot()
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
            return {
                "ok": True,
                "path": "/sdcard/" + dest_rel,
                "entries": len(ctx.player.playlist),
                "index": index,
                "index_ms": index_ms,
            }

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
            written = ctx.fs.write_bytes(rel, data)
            if rel.lower().startswith("odm/"):
                ctx.cache.invalidate()
            self.send_json(200, {"ok": True, "path": "/" + rel, "bytes": written})

        def h_files_delete(self, path):
            path, _, _ = self.path.partition("?")
            rel = card_path(percent_decode(path[len("/api/files/"):]))
            if rel is None:
                self.send_error_json(400, "path escapes the card")
                return
            with ctx.lock:
                active_rel = ctx.player.manifest_rel()
                active_idx = active_rel + ".idx"
                if rel == active_rel or rel == active_idx:
                    self.send_error_json(409, "that is the active manifest; select another set first")
                    return
                if is_manifest_path(rel):
                    ctx.fs.delete(rel + ".idx")
                existed = ctx.fs.delete(rel)
                if existed:
                    self.forget_copy("/" + rel)
                    if rel.lower().startswith("odm/"):
                        ctx.cache.invalidate()
            if existed:
                self.send_json(200, {"ok": True})
            else:
                self.send_error_json(404, enoent(rel))

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
            headers = {
                k: v for k, v in self.headers.items()
                if k.lower() not in ("host", "connection")
            }
            conn_cls = http.client.HTTPSConnection if parsed_target.scheme == "https" else http.client.HTTPConnection
            port = parsed_target.port or (443 if parsed_target.scheme == "https" else 80)
            conn = conn_cls(parsed_target.hostname, port, timeout=90)
            try:
                conn.request(method, self.path, body=body, headers=headers)
                resp = conn.getresponse()
                data = resp.read()
                self.send_response(resp.status, resp.reason or STATUS_TEXT.get(resp.status, ""))
                for hk in ("Content-Type", "Content-Encoding", "Cache-Control"):
                    v = resp.getheader(hk)
                    if v is not None:
                        self.send_header(hk, v)
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                if self.command != "HEAD":
                    self.wfile.write(data)
                self._resp_status = resp.status
                self._resp_bytes = len(data)
            finally:
                conn.close()

    return ProxyHandler


# --------------------------------------------------------------------------
# main
# --------------------------------------------------------------------------

def parse_args():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--root", help="an extracted card copy (gifs/, odm/, filelist.txt, ...)")
    ap.add_argument("--overlay", help="copy-on-write write layer in front of --root")
    ap.add_argument("--www", help="the web app to serve (default: <repo>/opendmd-web/www)")
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=8080)
    ap.add_argument("--dwell", type=float, default=5.0, help="seconds per now-playing item")
    ap.add_argument("--strict", action="store_true",
                    help="behave like the current firmware where the mock differs on purpose: "
                         "a '..' via the catch-all is a 404, stat/play cost a directory scan")
    ap.add_argument("--quiet", action="store_true", help="suppress the one-line-per-request log")
    ap.add_argument("--proxy", help="forward everything but --www to a real device at this URL")
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
