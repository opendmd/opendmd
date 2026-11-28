# /// script
# requires-python = ">=3.11"
# dependencies = ["pynacl==1.5.0", "aiohttp==3.12.15"]
# ///
"""A mock panel speaking Connect protocol v1 (docs/connect-protocol.md) to
a server (the mock server here, the Worker under `cf dev` later): it
registers, says hello, keeps a controller list, checks every signed
command with odc.Session (the panel's rules), pairs a guest through a
window it opens, acks, and sends status while watched; takes a GIF send
by the panel's rules (credit within `caps.win`, resume only for the
starting key, `size + copy_size` charged once, both hashes, the last
`credit`'s `done` + `name`, or `err`), keeping the files in memory (`landed`). A pretend
player underneath: no card, no display, no device.

    uv run tools/connect/mock_panel.py --server http://127.0.0.1:8787

Used in-process by roundtrip.py, which drives `open_pairing()` as the
LAN's `POST /api/connect/pair` would.
"""
from __future__ import annotations

import argparse
import asyncio
import hashlib
import os
import time

import re
from pathlib import Path

import aiohttp

import odc

REPO = Path(__file__).resolve().parents[2]


def bundled_formulas() -> list[tuple[str, str]]:
    """The firmware's built-in Formulas, as its `formulas` lists them:
    (stem, the `name "…"` line or the stem), the `-- bundle` files of
    opendmd-web/formulas/ in file-name order."""
    out = []
    for p in sorted((REPO / "opendmd-web" / "formulas").glob("*.psi")):
        src = p.read_text()
        if not re.search(r"^-- bundle", src, re.M):
            continue
        m = re.search(r'^\s*name\s+"([^"\n]*)"', src, re.M)
        out.append((p.stem, m.group(1) if m else p.stem))
    return out


def now_us() -> int:
    return int(time.monotonic() * 1_000_000)


class Panel:
    def __init__(self, server: str, fw: str = "mock-1.1.0", max_brightness: int = 128, version: int = odc.VERSION):
        self.server = server.rstrip("/")
        self.fw = fw
        self.version = version
        self.secret = os.urandom(32)
        self.dev: bytes | None = None
        self.keys: dict[bytes, odc.Key] = {}
        self.name = "Mock panel"
        self.max_brightness = max_brightness
        self.brightness = 64
        self.paused = False
        self.path = "/gifs/Arcade/demo.gif"
        self.set = ""
        self.window: odc.Window | None = None
        self.watched = False
        self.ws = None
        self.session: odc.Session | None = None
        self.connected = asyncio.Event()
        self.closed_with: int | None = None
        self.log: list[tuple] = []  # (k, n, c) accepted; ("refused", code) refused
        self.win = 2
        self.xfer: dict | None = None  # the held transfer
        self.quota_used = 0
        self.landed: dict[str, tuple[bytes, bytes]] = {}  # name -> (gif, copy)
        self.last_landed = None  # (k, args, name, when)
        # `formulas`: the built-in ones and a pretend card's ((stem, name);
        # None: no card); `formulas_cmd` False answers it as a panel up to
        # 1.1.0-beta.4 does (`not-allowed`).
        self.builtin_formulas = bundled_formulas()
        self.card_formulas: list[tuple[str, str]] | None = [("my-road", "My road"), ("den-clock", "Den clock")]
        self.formulas_cmd = True
        # `formulas`' own rate (5.6): a listing's first page one in this
        # many s per panel, its pages (after = the last `next`) free, 8 at most.
        self.formulas_every = 5.0
        self.formulas_first_at: float | None = None
        self.formulas_listing: list | None = None  # [next, at, pages]
        self.formulas_opens = 48  # card Formulas looked at per page
        # Formulas added by remote (5.7): the pretend card's files by stem
        # (source, compiled), the add under way, when the day's adds began
        # (in RAM, 16 in 24 h). Only a gallery-signed add gets here (odc's
        # parse_command checks the signature, as the core's parser does).
        self.abi = odc.GALLERY_ABI
        self.files: dict[str, tuple[bytes, bytes]] = {}
        self.add: dict | None = None
        self.add_started: list[float] = []

    async def register(self, http: aiohttp.ClientSession):
        async with http.post(f"{self.server}/v1/register", data=f'{{"v":{odc.VERSION},"secret_hash":"{odc.sha256(self.secret).hex()}"}}') as r:
            r.raise_for_status()
            self.dev = odc.parse_dev((await r.json())["dev"])

    def status(self) -> dict:
        return {
            "t": "status",
            "now_playing": {"path": self.path, "title": self.path.rsplit("/", 1)[-1], "elapsed_ms": 0, "seed": None, "sent": None},
            "paused": self.paused,
            "brightness": self.brightness,
            "max_brightness": self.max_brightness,
            "set": self.set,
            "fw": self.fw,
            "name": self.name,
        }

    def on_add(self, k, a):
        """`formula-add` as the firmware takes it (firmware/src/connect/fadd.rs)."""
        import hashlib
        h = self.add
        if h and h["k"] == k and h["a"] == a:
            if h.get("landed"):
                return {"done": True, "stem": h["target"]}
            if h["next"] == a["src_size"] + a["psb_size"]:
                return self.land()
            return {"next": h["next"]}
        if h and h["k"] != k and not h.get("landed") and time.monotonic() - h["at"] < 600:
            raise odc.Bad("busy")
        target = None
        for st in [a["stem"]] + [f"{a['stem'][:38]}-{i}" for i in range(2, 10)]:
            if st in self.files:
                if hashlib.sha256(self.files[st][0]).digest() == a["src_sha256"]:
                    return {"done": True, "stem": st}
                continue
            target = st
            break
        if target is None:
            raise odc.Bad("quota")
        now = time.monotonic()
        self.add_started = [t for t in self.add_started if now - t < odc.ADD_DAY]
        if len(self.add_started) >= odc.ADD_DAY_ADDS:
            raise odc.Bad("quota")
        self.add_started.append(now)
        self.add = {"k": k, "a": a, "target": target, "next": 0, "buf": b"", "at": now}
        return {"next": 0}

    def on_part(self, k, a):
        h = self.add
        if not h:
            raise odc.Bad("not-allowed")  # no add admitted
        if not h or h["a"]["id"] != a["id"] or h.get("landed"):
            raise odc.Bad("bad-arg")
        if h["k"] != k:
            raise odc.Bad("not-allowed")
        total = h["a"]["src_size"] + h["a"]["psb_size"]
        if a["off"] != h["next"] or a["off"] + len(a["data"]) > total:
            raise odc.Bad("bad-arg")
        h["buf"] += a["data"]
        h["next"] += len(a["data"])
        h["at"] = time.monotonic()
        if h["next"] < total:
            return {"next": h["next"]}
        return self.land()

    def land(self):
        import hashlib
        h, a = self.add, self.add["a"]
        src, psb = h["buf"][: a["src_size"]], h["buf"][a["src_size"]:]
        # (The firmware also loads every program in it through psb::verify.)
        if hashlib.sha256(src).digest() != a["src_sha256"] or hashlib.sha256(psb).digest() != a["psb_sha256"] or psb[:3] != b"PSB" or psb[3] != self.abi:
            self.add = None
            raise odc.Bad("bad-arg")
        self.files[h["target"]] = (src, psb)
        if self.card_formulas is not None:
            n = int.from_bytes(psb[4:6], "big")
            name = psb[6:6 + n].decode(errors="replace") or h["target"]
            self.card_formulas = [x for x in self.card_formulas if x[0] != h["target"]] + [(h["target"], name)]
        h["landed"] = True
        return {"done": True, "stem": h["target"]}

    def formulas_page(self, after):
        """One page of `formulas` (5.6) as the firmware answers it, or None (`rate`)."""
        now = time.monotonic()
        lst = self.formulas_listing
        if after is not None and lst and after == lst[0] and now - lst[1] <= 60 and lst[2] < 8:
            lst[2] += 1
        elif self.formulas_first_at is not None and now - self.formulas_first_at < self.formulas_every:
            return None
        else:
            self.formulas_first_at = now
            self.formulas_listing = lst = ["", now, 1]
        fl = odc.FormulaList(self.card_formulas is not None, self.abi)
        if after is None:
            for stem, name in self.builtin_formulas:
                fl.builtin(stem, name)
        cut, nxt = False, None
        if self.card_formulas is not None:
            cands = sorted((x for x in self.card_formulas if after is None or x[0] > after))
            cut = len(cands) > self.formulas_opens
            for stem, name in cands[: self.formulas_opens]:
                if not fl.card(stem, name):
                    cut = True
                    break
                nxt = stem
            if cut and nxt is None:
                nxt = after
        text = fl.finish(cut, nxt)
        r = odc.loads(text.encode())
        if r.get("more") and r.get("next"):
            lst[0], lst[1] = r["next"], now
        else:
            self.formulas_listing = None
        return r

    async def send(self, m) -> None:
        await self.ws.send_str(m if isinstance(m, str) else odc.dumps(m))

    async def send_keys(self):
        await self.send({"t": "keys", "keys": [{"k": k.k.hex(), "pub": k.pub.hex(), "f": k.flags} for k in self.keys.values()]})

    async def open_pairing(self) -> str:
        """The LAN's `POST /api/connect/pair`: a fresh secret, its window."""
        s = odc.b32(os.urandom(13), 100)
        self.window = odc.Window(s, until=time.monotonic() + odc.PAIR_TTL_S)
        await self.send({"t": "pair-open", "pair_id": odc.pair_id(s).hex(), "ttl": odc.PAIR_TTL_S})
        return s

    async def run(self, http: aiohttp.ClientSession, protocols=None):
        if self.dev is None:
            await self.register(http)
        protocols = protocols or (f"odc{self.version}", "d." + odc.dev_text(self.dev))
        sess = os.urandom(12)
        self.session = odc.Session(self.dev, sess, self.version, now_us())
        async with http.ws_connect(f"{self.server}/v1/panel", protocols=protocols, autoping=True, max_msg_size=4096) as ws:
            self.ws = ws
            await self.send({"t": "hello", "v": self.version, "secret": self.secret.hex(), "fw": self.fw, "sess": sess.hex(), "caps": {"send": True, "win": self.win}})
            await self.send_keys()
            await self.send({"t": "info", "name": self.name})
            self.connected.set()
            async for msg in ws:
                if msg.type == aiohttp.WSMsgType.BINARY:
                    await self.on_binary(msg.data)
                elif msg.type == aiohttp.WSMsgType.TEXT:
                    await self.on_text(msg.data)
            self.closed_with = ws.close_code
        self.connected.clear()

    async def on_text(self, text: str):
        if len(text.encode()) > odc.MAX_TEXT_IN:
            return
        try:
            m = odc.loads(text.encode())
        except odc.Bad:
            return
        if m.get("t") == "watch" and type(m.get("on")) is bool:
            self.watched = m["on"]
            if self.watched:
                await self.send(self.status())
        elif m.get("t") == "pair":
            await self.on_pair(m)

    async def on_pair(self, m: dict):
        g = m.get("g")
        pub = odc.unhex(m.get("pub"), 32)
        mac = odc.unhex(m.get("mac"), 32)
        raw = m.get("label")
        if not odc.is_int(g) or pub is None or mac is None or not isinstance(raw, str):
            return
        if self.window is None:
            await self.send({"t": "paired", "g": g, "err": "closed"})
            return
        try:
            mac2 = self.window.check(self.dev, pub, raw.encode(), mac, time.monotonic())
        except odc.Bad as e:
            await self.send({"t": "paired", "g": g, "err": str(e)})
            return
        # Cleaned only once the MAC, which covers it as sent, has checked.
        label = odc.clean_label(raw)
        if label is None or not odc.usable_public_key(pub) or len(self.keys) >= odc.MAX_KEYS or odc.key_id(pub) in self.keys:
            await self.send({"t": "paired", "g": g, "err": "bad-arg"})
            return
        # "Stored" (NVS) before the answer.
        key = odc.Key(pub, odc.FLAG_CAN_ADD, label, int(time.time()))
        self.keys[key.k] = key
        await self.send_keys()
        await self.send({"t": "pair-close", "pair_id": self.window.id().hex()})
        await self.send({"t": "paired", "g": g, "k": key.k.hex(), "mac2": mac2.hex()})

    def credit(self, x, err=None, name=None) -> dict:
        m = {"t": "credit", "id": x["a"]["id"], "upto": x["upto"], "k": x["k"].hex()}
        if err is not None:
            m["err"] = err
        if name is not None:
            m["done"], m["name"] = True, name
        return m

    async def on_chunk(self, frame: bytes):
        try:
            tid, off, data = odc.parse_chunk(frame)
        except odc.Bad:
            return
        x = self.xfer
        if x is None or tid != x["a"]["id"]:
            return
        if off != x["off"] or off + len(data) > x["upto"]:
            self.xfer = None
            await self.send(self.credit(x, "bad-arg"))
            return
        x["buf"] += data
        x["off"] += len(data)
        x["touched"] = time.monotonic()
        total = x["a"]["size"] + x["a"]["copy_size"]
        if x["off"] < total:
            nxt = min(x["off"] + self.win * odc.MAX_CHUNK, total)
            if nxt > x["upto"]:
                x["upto"] = nxt
                await self.send(self.credit(x))
            return
        self.xfer = None
        a = x["a"]
        gif, copy = bytes(x["buf"][: a["size"]]), bytes(x["buf"][a["size"]:])
        if hashlib.sha256(gif).digest() != a["sha256"] or (a["copy_size"] and hashlib.sha256(copy).digest() != a["copy_sha256"]) or not gif.startswith((b"GIF87a", b"GIF89a")):
            await self.send(self.credit(x, "bad-arg"))
            return
        name = a["name"]
        while name.lower() in (n.lower() for n in self.landed):
            name = odc.next_name(name)
        self.landed[name] = (gif, copy)
        self.last_landed = (x["k"], a, name, time.monotonic())
        path = f"/gifs/Sent/{name}"
        await self.send(self.credit(x, name=name))
        if a["play"]:
            self.path = path

    def on_send(self, k: bytes, a: dict):
        """(ack result, credit to say) or raises Bad(code)."""
        x = self.xfer
        now = time.monotonic()
        if x is not None and now - x["touched"] > odc.RESUME_S:
            x = self.xfer = None
        if x is not None:
            if x["k"] != k:
                raise odc.Bad("busy")
            if x["a"]["id"] == a["id"]:
                if x["a"] != a:
                    raise odc.Bad("bad-arg")
                # A resume: from the offset, nothing charged.
                x["upto"] = min(x["off"] + self.win * odc.MAX_CHUNK, a["size"] + a["copy_size"])
                x["touched"] = now
                return {"offset": x["off"], "resumed": True}, x
        l = self.last_landed
        if l is not None and l[0] == k and l[1] == a and now - l[3] <= odc.RESUME_S:
            total = a["size"] + a["copy_size"]
            return {"offset": total}, {"a": a, "k": k, "upto": total, "landed": l[2]}
        cost = a["size"] + a["copy_size"]
        if self.quota_used + cost > odc.QUOTA_BYTES:
            raise odc.Bad("quota")
        self.quota_used += cost
        x = self.xfer = {"a": a, "k": k, "off": 0, "upto": min(self.win * odc.MAX_CHUNK, cost), "buf": bytearray(), "touched": now}
        return {"offset": 0}, x

    async def on_binary(self, frame: bytes):
        if frame[:1] == bytes([odc.TYPE_CHUNK]):
            await self.on_chunk(frame)
            return
        try:
            k, n, c, a = self.session.check(self.keys, self.max_brightness, frame, now_us())
        except odc.Refusal as r:
            self.log.append(("refused", r.code))
            if r.ack() is not None:
                await self.send(r.ack())
            return
        self.log.append((k, n, c))
        r = None
        if c == "next":
            self.path = "/gifs/Arcade/next.gif"
        elif c == "prev":
            self.path = "/gifs/Arcade/prev.gif"
        elif c == "pause":
            self.paused = not self.paused
            r = {"paused": self.paused}
        elif c == "play":
            self.path = a["path"]
        elif c == "brightness":
            self.brightness = a["v"]
        elif c == "playlist":
            self.set = a["set"]
        elif c == "playlists":
            # As the firmware answers: {sets, more}, the main list ("") not in it.
            r = {"sets": ["consoles", "arcade"], "more": False}
        elif c == "formulas":
            if not self.formulas_cmd:
                await self.send(odc.Refusal("not-allowed", k, n).ack())
                return
            r = self.formulas_page(a.get("after"))
            if r is None:
                await self.send(odc.Refusal("rate", k, n).ack())
                return
        elif c == "controllers":
            r = [{"k": x.k.hex(), "label": x.label, "f": x.flags, "added": x.added, "added_by": x.added_by.hex() if x.added_by else None, "last_s": None} for x in self.keys.values()]
        elif c == "add-controller":
            nk = odc.Key(a["pub"], 0, a["label"], int(time.time()), k)
            self.keys[nk.k] = nk
            await self.send_keys()
        elif c == "remove-controller":
            self.keys.pop(a["k"], None)
            await self.send_keys()
        elif c == "name":
            self.name = a["name"]
            await self.send({"t": "info", "name": self.name})
        elif c in ("formula-add", "formula-part"):
            if not self.formulas_cmd:
                await self.send(odc.Refusal("not-allowed", k, n).ack())
                return
            try:
                r = self.on_add(k, a) if c == "formula-add" else self.on_part(k, a)
            except odc.Bad as e:
                await self.send(odc.Refusal(str(e), k, n).ack())
                return
        elif c == "send":
            try:
                r, x = self.on_send(k, a)
            except odc.Bad as e:
                await self.send(odc.Refusal(str(e), k, n).ack())
                return
            await self.send(odc.ack_ok(k, n, r))
            await self.send(self.credit(x, name=x.get("landed")))
            return
        elif c == "send-cancel":
            if self.xfer is not None and self.xfer["a"]["id"] == a["id"]:
                if self.xfer["k"] != k:
                    await self.send(odc.Refusal("not-allowed", k, n).ack())
                    return
                self.xfer = None
        await self.send(odc.ack_ok(k, n, r))
        if self.watched:
            await self.send(self.status())


async def amain(server: str):
    async with aiohttp.ClientSession() as http:
        p = Panel(server)
        await p.register(http)
        print("panel registered (its Connect id is kept in memory only)")
        await p.run(http)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--server", default="http://127.0.0.1:8787")
    asyncio.run(amain(ap.parse_args().server))


if __name__ == "__main__":
    main()
