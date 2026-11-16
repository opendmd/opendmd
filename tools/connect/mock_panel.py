# /// script
# requires-python = ">=3.11"
# dependencies = ["pynacl==1.5.0", "aiohttp==3.12.15"]
# ///
"""A mock panel speaking Connect protocol v1 (docs/connect-protocol.md) to
a server (the mock server here, the Worker under `cf dev` later): it
registers, says hello, keeps a controller list, checks every signed
command with odc.Session (the panel's rules), pairs a guest through a
window it opens, acks, and sends status while watched. A pretend player
underneath: no card, no display, no device.

    uv run tools/connect/mock_panel.py --server http://127.0.0.1:8787

Used in-process by roundtrip.py, which drives `open_pairing()` as the
LAN's `POST /api/connect/pair` would.
"""
from __future__ import annotations

import argparse
import asyncio
import os
import time

import aiohttp

import odc


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
            await self.send({"t": "hello", "v": self.version, "secret": self.secret.hex(), "fw": self.fw, "sess": sess.hex(), "caps": {"send": False, "win": 2}})
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
        if len(text.encode()) > odc.MAX_TEXT:
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
        label = odc.clean_label(m.get("label", "")) if isinstance(m.get("label"), str) else None
        if not odc.is_int(g) or pub is None or mac is None:
            return
        if self.window is None:
            await self.send({"t": "paired", "g": g, "err": "closed"})
            return
        try:
            mac2 = self.window.check(self.dev, pub, mac, time.monotonic())
        except odc.Bad as e:
            await self.send({"t": "paired", "g": g, "err": str(e)})
            return
        if label is None or not odc.usable_public_key(pub) or len(self.keys) >= odc.MAX_KEYS or odc.key_id(pub) in self.keys:
            await self.send({"t": "paired", "g": g, "err": "bad-arg"})
            return
        # "Stored" (NVS) before the answer.
        key = odc.Key(pub, odc.FLAG_CAN_ADD, label, int(time.time()))
        self.keys[key.k] = key
        await self.send_keys()
        await self.send({"t": "pair-close", "pair_id": self.window.id().hex()})
        await self.send({"t": "paired", "g": g, "k": key.k.hex(), "mac2": mac2.hex()})

    async def on_binary(self, frame: bytes):
        if frame[:1] == bytes([odc.TYPE_CHUNK]):
            return  # Sends are F4's; this mock advertises send: false.
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
            r = ["", "consoles", "arcade"]
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
        elif c in ("send", "send-cancel"):
            await self.send(odc.Refusal("busy", k, n).ack())
            return
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
