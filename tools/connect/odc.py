"""OpenDMD Connect protocol v1 in Python (docs/connect-protocol.md).

An implementation independent of the Rust core (opendmd-core/src/connect/),
shared by the mock panel, the mock server and the round trip, and checked
against the same vectors (opendmd-core/tests/connect-vectors.json) by
vectors_check.py. Imported by those scripts, never run on its own; needs
pynacl.

Nothing here touches the network or a device.
"""
from __future__ import annotations

import hashlib
import hmac
import json
import unicodedata
from dataclasses import dataclass, field

import nacl.bindings
import nacl.exceptions
import nacl.signing

VERSION = 1
MIN_VERSION = VERSION - 1 if VERSION > 1 else 1

CLOSE_TOO_OLD = 4426
CLOSE_TOO_NEW = 4406
CLOSE_STAY_AWAY = 4429
CLOSE_REFUSED = 4403

TYPE_COMMAND = 0x01
TYPE_CHUNK = 0x02
COMMAND_HEAD = 1 + 8 + 64
MAX_BODY = 700
MAX_TEXT_IN = 1024  # what anyone but the panel sends: the panel's receive buffer
MAX_TEXT_OUT = 2048  # what the panel sends (a full controllers ack)
MAX_CHUNK = 1024
MAX_FILE = 512 * 1024
# A send (docs/connect-protocol.md 8): the panel's quota, its resume window.
QUOTA_BYTES = 8 * 1024 * 1024
RESUME_S = 600
MAX_KEYS = 8
MAX_LABEL = 24
MAX_NAME_CHARS = 32
MAX_PATH = 240
MAX_SAFE = (1 << 53) - 1
MAX_DEPTH = 4
MAX_COUNTERS = 16
RATE_PER_KEY = 4
RATE_PER_PANEL = 8
FLAG_CAN_ADD = 1
PAIR_TTL_S = 300
PAIR_TRIES = 3

CMD_DOMAIN = b"opendmd-connect-v1 cmd\0"
ATTACH_DOMAIN = b"opendmd-connect-v1 attach\0"
DEV_TAG_DOMAIN = b"opendmd-connect-v1 dev\0"
PAIR_ID_DOMAIN = b"opendmd pair id v1"
PAIR_MAC_DOMAIN = b"opendmd pair v1"
PAIRED_MAC_DOMAIN = b"opendmd paired v1"
INVITE_ID_DOMAIN = b"opendmd invite id v1"
ADD_MAC_DOMAIN = b"opendmd add v1"
CONFIRM_DOMAIN = b"opendmd confirm v1"

ALLOWED = (
    "play", "next", "prev", "pause", "brightness", "playlist", "playlists", "formulas",
    "formula-add", "formula-part", "send", "send-cancel", "controllers", "add-controller", "remove-controller", "name",
)

# --- Encodings -----------------------------------------------------------------

B32 = "abcdefghijklmnopqrstuvwxyz234567"


def b32(data: bytes, bits: int | None = None) -> str:
    bits = len(data) * 8 if bits is None else bits
    n = int.from_bytes(data, "big") if data else 0
    total = len(data) * 8
    out = []
    for c in range(-(-bits // 5)):
        v = 0
        for b in range(5):
            bit = c * 5 + b
            on = bit < bits and bit < total and (n >> (total - 1 - bit)) & 1
            v = (v << 1) | int(bool(on))
        out.append(B32[v])
    return "".join(out)


def unb32(s: str, nbytes: int, bits: int | None = None) -> bytes | None:
    bits = nbytes * 8 if bits is None else bits
    if not isinstance(s, str) or len(s) != -(-bits // 5) or nbytes * 8 < bits:
        return None
    acc = 0
    for ch in s:
        i = B32.find(ch)
        if i < 0:
            return None
        acc = (acc << 5) | i
    pad = len(s) * 5 - bits
    if acc & ((1 << pad) - 1):
        return None
    acc >>= pad
    return (acc << (nbytes * 8 - bits)).to_bytes(nbytes, "big")


def unhex(s, n: int) -> bytes | None:
    if not isinstance(s, str) or len(s) != 2 * n or any(c not in "0123456789abcdef" for c in s):
        return None
    return bytes.fromhex(s)


def sha256(*parts: bytes) -> bytes:
    h = hashlib.sha256()
    for p in parts:
        h.update(p)
    return h.digest()


def mac256(key: bytes, *parts: bytes) -> bytes:
    return hmac.new(key, b"".join(parts), hashlib.sha256).digest()


def key_id(pub: bytes) -> bytes:
    return sha256(pub)[:8]


# --- Strict JSON ---------------------------------------------------------------


class Bad(Exception):
    pass


def _no_dupes(pairs):
    seen = set()
    for k, _ in pairs:
        if k in seen:
            raise Bad("duplicate key")
        seen.add(k)
    return dict(pairs)


def _int(s: str):
    if s == "-0" or len(s.lstrip("-")) > 16:
        raise Bad("number")
    n = int(s)
    if abs(n) > MAX_SAFE:
        raise Bad("number")
    return n


def _float(_s):
    raise Bad("number")


def _walk(v, depth: int):
    if isinstance(v, (dict, list)):
        depth += 1
        if depth > MAX_DEPTH:
            raise Bad("too deep")
        items = v.items() if isinstance(v, dict) else enumerate(v)
        for k, x in items:
            if isinstance(k, str):
                _check_str(k)
            _walk(x, depth)
    elif isinstance(v, str):
        _check_str(v)


def _check_str(s: str):
    if "\0" in s:
        raise Bad("NUL")
    s.encode("utf-8")  # a lone surrogate raises


def loads(data: bytes):
    """One JSON object, read as strictly as the Rust core reads it."""
    try:
        text = data.decode("utf-8")
        v = json.loads(text, object_pairs_hook=_no_dupes, parse_int=_int, parse_float=_float, parse_constant=_float)
        _walk(v, 0)
    except (Bad, ValueError, UnicodeError, RecursionError) as e:
        raise Bad(str(e)) from None
    if not isinstance(v, dict):
        raise Bad("not an object")
    return v


def dumps(v) -> str:
    return json.dumps(v, separators=(",", ":"), ensure_ascii=False)


def is_int(v) -> bool:
    return type(v) is int


# --- Ids, versions ---------------------------------------------------------------


def parse_subprotocol(s: str):
    if not s.startswith("odc"):
        return None
    d = s[3:]
    if not d or len(d) > 3 or d[0] == "0" or not all(c in "0123456789" for c in d):
        return None
    return int(d)


def negotiate(offered):
    best, low, high = None, False, False
    for s in offered:
        v = parse_subprotocol(s.strip())
        if v is None:
            continue
        if v < MIN_VERSION:
            low = True
        elif v > VERSION:
            high = True
        elif best is None or v > best:
            best = v
    if best is not None:
        return f"accept:{best}"
    if low:
        return f"too-old:{MIN_VERSION}"
    if high:
        return f"too-new:{VERSION}"
    return "none"


def mint_dev(random10: bytes, id_key: bytes) -> bytes:
    return random10 + mac256(id_key, DEV_TAG_DOMAIN, random10)[:6]


def dev_tag_ok(dev: bytes, id_key: bytes) -> bool:
    return len(dev) == 16 and hmac.compare_digest(mac256(id_key, DEV_TAG_DOMAIN, dev[:10])[:6], dev[10:])


def dev_text(dev: bytes) -> str:
    return b32(dev)


def parse_dev(s) -> bytes | None:
    return unb32(s, 16)


# --- Signatures ------------------------------------------------------------------


def usable_public_key(pub: bytes) -> bool:
    try:
        return len(pub) == 32 and nacl.bindings.crypto_core_ed25519_is_valid_point(pub)
    except Exception:
        return False


def verify(pub: bytes, msg: bytes, sig: bytes) -> bool:
    if not usable_public_key(pub):
        return False
    try:
        nacl.signing.VerifyKey(pub).verify(msg, sig)
        return True
    except (nacl.exceptions.BadSignatureError, ValueError, TypeError):
        return False


def command_body(dev: bytes, sess: bytes, n: int, c: str, a: dict | None = None) -> bytes:
    return dumps({"v": VERSION, "dev": dev_text(dev), "sess": sess.hex(), "n": n, "c": c, "a": a or {}}).encode()


def sign_command(sk: nacl.signing.SigningKey, body: bytes) -> bytes:
    sig = sk.sign(CMD_DOMAIN + body).signature
    return bytes([TYPE_COMMAND]) + key_id(bytes(sk.verify_key)) + sig + body


def split_command(frame: bytes):
    """(k, sig, body), or raises Refusal('bad-frame' / 'too-big')."""
    if len(frame) < COMMAND_HEAD or frame[0] != TYPE_COMMAND:
        raise Refusal("bad-frame", None)
    k = frame[1:9]
    if len(frame) - COMMAND_HEAD > MAX_BODY:
        raise Refusal("too-big", k)
    return k, frame[9:COMMAND_HEAD], frame[COMMAND_HEAD:]


def precheck(keys: dict[bytes, bytes], frame: bytes) -> bytes:
    """The server's check: a known key and a good signature. keys: k -> pub."""
    k, sig, body = split_command(frame)
    pub = keys.get(k)
    if pub is None:
        raise Refusal("unknown-key", k)
    if not verify(pub, CMD_DOMAIN + body, sig):
        raise Refusal("bad-sig", k)
    return k


def attach_signature(sk, dev: bytes, nonce: bytes) -> bytes:
    return sk.sign(ATTACH_DOMAIN + dev + nonce).signature


def verify_attach(pub: bytes, dev: bytes, nonce: bytes, sig: bytes) -> bool:
    return verify(pub, ATTACH_DOMAIN + dev + nonce, sig)


# --- Pairing ---------------------------------------------------------------------


def pair_id(s: str) -> bytes:
    return sha256(PAIR_ID_DOMAIN, s.encode())[:10]


def invite_id(s: str) -> bytes:
    return sha256(INVITE_ID_DOMAIN, s.encode())[:10]


def pair_mac(s: str, dev: bytes, pub: bytes, label: bytes) -> bytes:
    """label: its UTF-8 bytes as sent (after JSON unescaping, before cleaning), last."""
    return mac256(s.encode(), PAIR_MAC_DOMAIN, dev, pub, label)


def paired_mac(s: str, dev: bytes, pub: bytes) -> bytes:
    return mac256(s.encode(), PAIRED_MAC_DOMAIN, dev, pub)


def add_mac(s: str, dev: bytes, pub: bytes, label: bytes) -> bytes:
    return mac256(s.encode(), ADD_MAC_DOMAIN, dev, pub, label)


def confirm_code(pub: bytes) -> str:
    return b32(sha256(CONFIRM_DOMAIN, pub), 20).upper()


@dataclass
class Window:
    secret: str
    until: float
    invite: bool = False
    tries: int = PAIR_TRIES
    open: bool = True

    def id(self) -> bytes:
        return invite_id(self.secret) if self.invite else pair_id(self.secret)

    def check(self, dev: bytes, pub: bytes, label: bytes, mac: bytes, now: float) -> bytes:
        """mac2, or raises Bad('closed' / 'expired' / 'bad-mac')."""
        if not self.open:
            raise Bad("closed")
        if now >= self.until:
            self.open = False
            raise Bad("expired")
        want = add_mac(self.secret, dev, pub, label) if self.invite else pair_mac(self.secret, dev, pub, label)
        if not hmac.compare_digest(want, mac):
            self.tries -= 1
            if self.tries <= 0:
                self.open = False
            raise Bad("bad-mac")
        self.open = False
        return paired_mac(self.secret, dev, pub)


# --- Text rules ------------------------------------------------------------------

_INVISIBLE = [
    (0x00AD, 0x00AD), (0x034F, 0x034F), (0x061C, 0x061C), (0x115F, 0x1160), (0x17B4, 0x17B5),
    (0x180E, 0x180E), (0x200B, 0x200F), (0x2028, 0x202E), (0x2060, 0x2064), (0x2066, 0x206F),
    (0x3164, 0x3164), (0xFE00, 0xFE0F), (0xFEFF, 0xFEFF), (0xFFA0, 0xFFA0), (0xFFF0, 0xFFFB),
    (0xE0000, 0xE0FFF),
]


def _control(c: str) -> bool:
    return unicodedata.category(c) == "Cc"


def _invisible(c: str) -> bool:
    o = ord(c)
    return _control(c) or any(a <= o <= b for a, b in _INVISIBLE)


def clean_text(s: str, max_bytes: int, max_chars: int):
    t = "".join(c for c in s if not _invisible(c)).strip()
    if not t or len(t.encode()) > max_bytes or len(t) > max_chars:
        return None
    return t


def clean_label(s: str):
    return clean_text(s, MAX_LABEL, MAX_LABEL)


def _segment_ok(s: str) -> bool:
    return bool(s) and not s.startswith(".") and not s.endswith((".", " ")) and not any(_control(c) or c in '\\?#:*<>|"' for c in s)


def _stem_ok(s: str) -> bool:
    return 1 <= len(s.encode()) <= 64 and s[0].isascii() and s[0].isalnum() and all(c in "abcdefghijklmnopqrstuvwxyz0123456789-_" for c in s)


def remote_play_path_ok(p: str) -> bool:
    if p.startswith("@formula:"):
        return _stem_ok(p[len("@formula:"):])
    if p.startswith("/formulas/") and p.endswith(".psi"):
        return _stem_ok(p[len("/formulas/"):-4])
    if not p.startswith("/gifs/"):
        return False
    rest = p[len("/gifs/"):]
    rb = rest.encode()
    return len(p.encode()) <= MAX_PATH and len(rb) > 4 and rb[-4:].lower() == b".gif" and all(_segment_ok(x) for x in rest.split("/"))


def remote_set_ok(s: str) -> bool:
    return s == "" or (len(s.encode()) <= 64 and _segment_ok(s) and "/" not in s and "\0" not in s and ".." not in s)


def sent_name_ok(s: str) -> bool:
    b = s.encode()
    utf16 = len(s.encode("utf-16-le")) // 2
    return len(s) <= 120 and len(b) <= 229 and utf16 + 19 <= 255 and len(b) > 4 and b[-4:].lower() == b".gif" and _segment_ok(s) and "/" not in s


# --- The panel's checks ------------------------------------------------------------


class Refusal(Exception):
    def __init__(self, code: str, k: bytes | None = None, n: int | None = None):
        super().__init__(code)
        self.code, self.k, self.n = code, k, n

    def ack(self):
        if self.k is None:
            return None
        m = {"t": "ack", "k": self.k.hex()}
        if self.n is not None:
            m["n"] = self.n
        m["err"] = self.code
        return dumps(m)


class Bucket:
    def __init__(self, per_s: int, now_us: int):
        self.per_s, self.micro, self.last = per_s, per_s * 1_000_000, now_us

    def take(self, now_us: int) -> bool:
        cap = self.per_s * 1_000_000
        dt = max(0, now_us - self.last)
        self.micro = min(cap, self.micro + dt * self.per_s)
        self.last = max(self.last, now_us)
        if self.micro >= 1_000_000:
            self.micro -= 1_000_000
            return True
        return False


@dataclass
class Key:
    pub: bytes
    flags: int
    label: str
    added: int = 0
    added_by: bytes | None = None

    @property
    def k(self) -> bytes:
        return key_id(self.pub)


def _u32(a, name):
    v = a.get(name)
    if not is_int(v) or not 0 <= v <= 0xFFFFFFFF:
        raise Bad("bad-arg")
    return v


def _str(a, name):
    v = a.get(name)
    if not isinstance(v, str):
        raise Bad("bad-arg")
    return v


def _send_args(a):
    def no(code="bad-arg"):
        raise Bad(code)
    id_ = _u32(a, "id")
    name = _str(a, "name")
    size = a.get("size")
    if not is_int(size):
        no()
    copy = a.get("copy_size", 0) if "copy_size" in a else 0
    if not is_int(copy):
        no()
    if size <= 0 or copy < 0:
        no()
    if size > MAX_FILE or copy > MAX_FILE:
        no("too-big")
    if id_ == 0 or not sent_name_ok(name):
        no()
    sha = unhex(_str(a, "sha256"), 32) or no()
    if copy == 0:
        if "copy_sha256" in a:
            no()
        csha = None
    else:
        if "copy_sha256" not in a:
            no()
        csha = unhex(a["copy_sha256"], 32) or no()
    play = a.get("play", False) if "play" in a else False
    if type(play) is not bool:
        no()
    return {"id": id_, "name": name, "size": size, "sha256": sha, "copy_size": copy, "copy_sha256": csha, "play": play}


def parse_command(c: str, a: dict, max_brightness: int, can_add: bool, keys: dict[bytes, Key]):
    """(c, args) validated; raises Bad(code)."""
    if c == "play":
        p = _str(a, "path")
        if not remote_play_path_ok(p):
            raise Bad("bad-arg")
        return c, {"path": p}
    if c == "formulas":
        # Strict (5.6): only `after`, a stem, so a later page can tell a panel without a feature.
        if set(a) - {"after"} or ("after" in a and not (isinstance(a["after"], str) and _stem_ok(a["after"]))):
            raise Bad("bad-arg")
        return c, {"after": a.get("after")}
    if c == "formula-add":
        args = add_args(a)
        if not ADDS_TAKEN:
            raise Bad("not-allowed")
        return c, args
    if c == "formula-part":
        return c, part_args(a)
    if c in ("next", "prev", "pause", "playlists", "controllers"):
        return c, {}
    if c == "brightness":
        v = a.get("v")
        if not is_int(v) or not 0 <= v <= max_brightness:
            raise Bad("bad-arg")
        return c, {"v": v}
    if c == "playlist":
        s = _str(a, "set")
        if not remote_set_ok(s):
            raise Bad("bad-arg")
        return c, {"set": s}
    if c == "send":
        return c, _send_args(a)
    if c == "send-cancel":
        return c, {"id": _u32(a, "id")}
    if c == "add-controller":
        if not can_add:
            raise Bad("not-allowed")
        pub = unhex(_str(a, "pub"), 32)
        if pub is None or not usable_public_key(pub) or key_id(pub) in keys:
            raise Bad("bad-arg")
        label = clean_label(_str(a, "label"))
        if label is None:
            raise Bad("bad-arg")
        if len(keys) >= MAX_KEYS:
            raise Bad("quota")
        return c, {"pub": pub, "label": label}
    if c == "remove-controller":
        k = unhex(_str(a, "k"), 8)
        if k is None or k not in keys:
            raise Bad("bad-arg")
        return c, {"k": k}
    if c == "name":
        n = clean_text(_str(a, "name"), 4 * MAX_NAME_CHARS, MAX_NAME_CHARS)
        if n is None:
            raise Bad("bad-arg")
        return c, {"name": n}
    raise Bad("not-allowed")


@dataclass
class Session:
    """One connection's state on the panel."""

    dev: bytes
    sess: bytes
    version: int = VERSION
    now_us: int = 0
    counters: dict = field(default_factory=dict)  # k -> [last n, Bucket]

    def __post_init__(self):
        self.panel_rate = Bucket(RATE_PER_PANEL, self.now_us)

    def check(self, keys: dict[bytes, Key], max_brightness: int, frame: bytes, now_us: int = 0):
        """(k, n, c, args), or raises Refusal."""
        k, sig, body = split_command(frame)
        entry = keys.get(k)
        if entry is None:
            raise Refusal("unknown-key", k)
        if not self.panel_rate.take(now_us):
            raise Refusal("rate", k)
        if not verify(entry.pub, CMD_DOMAIN + body, sig):
            raise Refusal("bad-sig", k)
        try:
            b = loads(body)
        except Bad:
            raise Refusal("bad-arg", k) from None
        n = b.get("n")
        n = n if is_int(n) and n >= 1 else None

        def no(code):
            return Refusal(code, k, n)

        if n is None:
            raise no("bad-arg")
        if not (is_int(b.get("v")) and b["v"] == self.version):
            raise no("bad-arg")
        if parse_dev(b.get("dev")) != self.dev:
            raise no("stale-session")
        if unhex(b.get("sess"), 12) != self.sess:
            raise no("stale-session")
        c = b.get("c")
        if not isinstance(c, str):
            raise no("bad-arg")
        a = b.get("a", {}) if "a" in b else {}
        if not isinstance(a, dict):
            raise no("bad-arg")
        slot = self.counters.get(k)
        if slot is None:
            if len(self.counters) >= MAX_COUNTERS:
                raise no("busy")
            slot = self.counters[k] = [0, Bucket(RATE_PER_KEY, now_us)]
        if n <= slot[0]:
            raise no("replay")
        slot[0] = n
        if not slot[1].take(now_us):
            raise no("rate")
        try:
            c, args = parse_command(c, a, max_brightness, bool(entry.flags & FLAG_CAN_ADD), keys)
        except Bad as e:
            raise no(str(e)) from None
        return k, n, c, args


# What an ack adds around its result (t, a key id, the largest n, ok): a
# bounded result (playlists, formulas) takes at most RESULT_ROOM bytes.
ACK_OVERHEAD = 100
RESULT_ROOM = MAX_TEXT_OUT - ACK_OVERHEAD


# --- Formulas added by remote (5.7; opendmd_core::connect::fadd) ------------------

ADD_MAX_STEM = 40
ADD_MAX_SOURCE = 16 * 1024
ADD_MIN_PSB, ADD_MAX_PSB = 6, 8 * 1024
ADD_MAX_PART = 384
ADD_QUOTA_ADDS, ADD_QUOTA_BYTES = 16, 256 * 1024
# Whether a formula-add is admitted (the core: only with the witness
# opendmd_core::gallery_sig::check makes, and the command carries no
# signature yet): False, every formula-add refused not-allowed after its
# argument rules; a part with no add held is the panel's not-allowed. Only a
# test of the mock's own add flow sets it (roundtrip.py), never a panel.
ADDS_TAKEN = False


def add_stem_ok(s) -> bool:
    return isinstance(s, str) and 1 <= len(s) <= ADD_MAX_STEM and s[0].isascii() and s[0].isalnum() and all(c in "abcdefghijklmnopqrstuvwxyz0123456789-" for c in s)


def b64url(b: bytes) -> str:
    import base64
    return base64.urlsafe_b64encode(b).decode().rstrip("=")


def unb64url(s: str):
    import base64
    import re
    if not isinstance(s, str) or len(s) % 4 == 1 or not re.fullmatch(r"[A-Za-z0-9_-]*", s):
        return None
    b = base64.urlsafe_b64decode(s + "=" * (-len(s) % 4))
    return b if b64url(b) == s else None  # canonical only


def add_args(a: dict) -> dict:
    def no(code="bad-arg"):
        raise Bad(code)
    i = a.get("id")
    if not is_int(i) or not 0 < i <= 0xFFFFFFFF or not add_stem_ok(a.get("stem")) or not isinstance(a.get("name"), str):
        no()
    name = clean_text(a["name"], 240, 60)
    ss, ps = a.get("src_size"), a.get("psb_size")
    if not is_int(ss) or not is_int(ps):
        no()
    if ss > ADD_MAX_SOURCE or ps > ADD_MAX_PSB:
        no("too-big")
    if ss < 1 or ps < ADD_MIN_PSB or name is None:
        no()
    sh, ph = unhex(a.get("src_sha256"), 32), unhex(a.get("psb_sha256"), 32)
    if sh is None or ph is None:
        no()
    return {"id": i, "stem": a["stem"], "name": name, "src_size": ss, "src_sha256": sh, "psb_size": ps, "psb_sha256": ph}


def part_args(a: dict) -> dict:
    i, off, d = a.get("id"), a.get("off"), a.get("data")
    if not is_int(i) or not 0 < i <= 0xFFFFFFFF or not is_int(off) or not 0 <= off <= 0xFFFFFFFF or not isinstance(d, str):
        raise Bad("bad-arg")
    if len(d) > -(-ADD_MAX_PART // 3) * 4:
        raise Bad("too-big")
    b = unb64url(d)
    if not b or len(b) > ADD_MAX_PART:
        raise Bad("bad-arg")
    return {"id": i, "off": off, "data": b}


def quote(s: str) -> str:
    """A JSON string as the panel writes one (opendmd_core::connect::json::push_str):
    `"`, `\\`, `\n`, `\r`, `\t` escaped, other controls and U+007F as \\u00xx,
    everything else as it is."""
    out = ['"']
    for ch in s:
        if ch == '"':
            out.append('\\"')
        elif ch == "\\":
            out.append("\\\\")
        elif ch == "\n":
            out.append("\\n")
        elif ch == "\r":
            out.append("\\r")
        elif ch == "\t":
            out.append("\\t")
        elif ord(ch) < 0x20 or ch == "\x7f":
            out.append("\\u%04x" % ord(ch))
        else:
            out.append(ch)
    out.append('"')
    return "".join(out)


_CARD_OPEN = '],"card":['
_CLOSE_CARD_MAX = 1 + len(',"more":true,"next":""}') + 64
_CLOSE_NULL = '],"card":null,"more":false}'


class FormulaList:
    """`formulas`'s result as the panel builds it (opendmd_core::connect::proto::FormulaList,
    protocol 5.6), byte for byte: built-in ones, then the card's, while the
    whole stays within RESULT_ROOM bytes whatever the closing; the first
    that does not fit and every one after it are left out (`more`)."""

    def __init__(self, card: bool, abi: int | None = None):
        head = "{" if abi is None else '{"abi":%d,' % abi
        self.s, self.has_card, self.card_open, self.more = head + '"builtin":[', card, False, False

    def _closing(self) -> int:
        if not self.has_card:
            return len(_CLOSE_NULL)
        return _CLOSE_CARD_MAX + (0 if self.card_open else len(_CARD_OPEN))

    def _push(self, card: bool, stem: str, name: str) -> bool:
        if self.more or (card and not self.has_card):
            self.more = self.more or (card and not self.has_card)
            return False
        opening = _CARD_OPEN if card and not self.card_open else ""
        e = opening + ("," if not self.s.endswith("[") and not opening else "") + "[" + quote(stem) + "," + quote(name) + "]"
        after = _CLOSE_CARD_MAX if card or self.card_open else self._closing()
        if len(self.s.encode()) + len(e.encode()) + after > RESULT_ROOM:
            self.more = True
            return False
        self.s += e
        self.card_open = self.card_open or card
        return True

    def builtin(self, stem: str, name: str) -> bool:
        return False if self.card_open else self._push(False, stem, name)

    def card(self, stem: str, name: str) -> bool:
        return self._push(True, stem, name)

    def finish(self, cut: bool = False, next_: str | None = None) -> str:
        more = self.has_card and (self.more or cut)
        if not self.has_card:
            return self.s + _CLOSE_NULL
        s = self.s + ("" if self.card_open else _CARD_OPEN) + "]"
        if more and next_ is not None and _stem_ok(next_):
            return s + ',"more":true,"next":"' + next_ + '"}'
        return s + (',"more":true}' if more else ',"more":false}')


def formulas_result(builtin, card, cut: bool = False, next_: str | None = None) -> str:
    """`formulas`'s result text (FormulaList): `card` None for no card."""
    lst = FormulaList(card is not None)
    for stem, name in builtin:
        if not lst.builtin(stem, name):
            break
    for stem, name in card or ():
        if not lst.card(stem, name):
            break
    return lst.finish(cut, next_)


def ack_ok(k: bytes, n: int, r=None) -> str:
    m = {"t": "ack", "k": k.hex(), "n": n, "ok": True}
    if r is not None:
        m["r"] = r
    return dumps(m)


# --- Chunks ----------------------------------------------------------------------


def next_name(n: str) -> str:
    """The next name to try when one is taken (the web app's nextName,
    opendmd_core::connect::xfer::next_name)."""
    import re
    m = re.match(r"^(.*_)(\d+)((?:_[^_]*)?\.gif)$", n, re.I)
    if m:
        return m[1] + str(int(m[2]) + 1).zfill(len(m[2])) + m[3]
    return re.sub(r"(\.gif)$", r"_2\1", n, flags=re.I)


def chunk_frame(tid: int, offset: int, data: bytes) -> bytes:
    return bytes([TYPE_CHUNK]) + tid.to_bytes(4, "big") + offset.to_bytes(4, "big") + data


def parse_chunk(frame: bytes):
    if len(frame) <= 9 or len(frame) > 9 + MAX_CHUNK or frame[0] != TYPE_CHUNK:
        raise Bad("bad-frame")
    return int.from_bytes(frame[1:5], "big"), int.from_bytes(frame[5:9], "big"), frame[9:]
