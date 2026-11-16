# /// script
# requires-python = ">=3.11"
# dependencies = ["pynacl==1.5.0", "aiohttp==3.12.15"]
# ///
"""A local stand-in for the Connect Worker and its objects (P1-PLAN 3),
speaking protocol v1 (docs/connect-protocol.md) over plain HTTP and
WebSocket on localhost. For the round trip (roundtrip.py) and, later, the
emulator lane; never deployed, never connects anywhere.

    uv run tools/connect/mock_server.py --listen 127.0.0.1:8787

Routes: POST /v1/register, GET /v1/panel (upgrade), GET /v1/ctl (upgrade),
POST /v1/find, GET /v1/health; everything else 404. State is in memory and
an `ID_KEY` drawn at start. What it enforces, as the Worker will: the id's
tag before anything, version negotiation (4426 / 4406), the panel's secret
(4403), the controller's challenge and signed attach (4403 alike for an
unknown key and a bad signature), guests only with an open window, the
signature pre-check and rates on commands, routing of ack / credit /
paired, `watch` when controllers come and go. It keeps no status and no
history.
"""
from __future__ import annotations

import argparse
import asyncio
import os
import time
from dataclasses import dataclass, field

from aiohttp import WSMsgType, web

import odc

ORIGIN = "https://opendmd.dev"
GUEST_LIMIT_S = 60


def now_us() -> int:
    return int(time.monotonic() * 1_000_000)


@dataclass
class Record:
    secret_hash: bytes
    keys: dict = field(default_factory=dict)  # k -> (pub, flags)
    name: str = ""
    fw: str = ""
    panel: web.WebSocketResponse | None = None
    sess: str | None = None
    ctls: dict = field(default_factory=dict)  # ws -> k
    guests: dict = field(default_factory=dict)  # g -> (ws, window id, inviter ws or None)
    next_g: int = 1
    pairs: dict = field(default_factory=dict)  # pair_id -> expiry (monotonic s)
    invites: dict = field(default_factory=dict)  # invite_id -> (inviter ws, expiry)
    transfers: dict = field(default_factory=dict)  # transfer id -> ctl ws
    rates: dict = field(default_factory=dict)  # k -> Bucket
    panel_rate: odc.Bucket = field(default_factory=lambda: odc.Bucket(odc.RATE_PER_PANEL, now_us()))


class Server:
    def __init__(self, id_key: bytes | None = None):
        self.id_key = id_key or os.urandom(32)
        self.panels: dict[bytes, Record] = {}
        self.finds: dict[bytes, tuple[bytes, float]] = {}  # pair_id -> (dev, expiry)
        # Test hooks: counts of what was refused, by reason; `rewrite_label`
        # makes this a hostile relay that changes the label in a relayed
        # `pair` or `add-req`.
        self.rewrite_label: str | None = None
        self.refused: dict[str, int] = {}

    def refuse(self, why: str):
        self.refused[why] = self.refused.get(why, 0) + 1

    def app(self) -> web.Application:
        app = web.Application()
        app.router.add_post("/v1/register", self.register)
        app.router.add_get("/v1/panel", self.panel_socket)
        app.router.add_get("/v1/ctl", self.ctl_socket)
        app.router.add_post("/v1/find", self.find)
        app.router.add_get("/v1/health", self.health)
        return app

    # --- HTTP --------------------------------------------------------------

    async def health(self, _req):
        return web.json_response({"ok": True, "v": odc.VERSION, "min_v": odc.MIN_VERSION})

    async def register(self, req: web.Request):
        try:
            body = odc.loads(await req.read())
        except odc.Bad:
            return web.Response(status=400)
        h = odc.unhex(body.get("secret_hash"), 32)
        v = body.get("v")
        if h is None or not odc.is_int(v):
            return web.Response(status=400)
        if v < odc.MIN_VERSION:
            return web.json_response({"min_v": odc.MIN_VERSION}, status=426)
        dev = odc.mint_dev(os.urandom(10), self.id_key)
        self.panels[dev] = Record(secret_hash=h)
        return web.json_response({"dev": odc.dev_text(dev)})

    async def find(self, req: web.Request):
        try:
            body = odc.loads(await req.read())
        except odc.Bad:
            return web.Response(status=400)
        pid = odc.unhex(body.get("pair_id"), 10)
        hit = self.finds.get(pid) if pid else None
        if not hit or hit[1] <= time.monotonic():
            self.refuse("find")
            return web.Response(status=404)
        return web.json_response({"dev": odc.dev_text(hit[0])})

    # --- The upgrade gate ----------------------------------------------------

    async def gate(self, req: web.Request):
        """(ws, record, dev) once upgraded, or a Response refusing it."""
        offered = [s.strip() for s in req.headers.get("Sec-WebSocket-Protocol", "").split(",") if s.strip()]
        devs = [d for d in (odc.parse_dev(s[2:]) for s in offered if s.startswith("d.")) if d]
        if len(devs) != 1 or not odc.dev_tag_ok(devs[0], self.id_key):
            self.refuse("made-up-id")
            return None, web.Response(status=404)
        dev = devs[0]
        rec = self.panels.get(dev)
        if rec is None:
            self.refuse("unknown-id")
            return None, web.Response(status=404)
        verdict = odc.negotiate(offered)
        if verdict == "none":
            return None, web.Response(status=400)
        if verdict.startswith("accept:"):
            ws = web.WebSocketResponse(protocols=(f"odc{verdict[7:]}",), max_msg_size=4096, autoping=True)
            await ws.prepare(req)
            return (ws, rec, dev), None
        ws = web.WebSocketResponse(max_msg_size=4096)
        await ws.prepare(req)
        kind, v = verdict.split(":")
        self.refuse(kind)
        await ws.close(code=odc.CLOSE_TOO_OLD if kind == "too-old" else odc.CLOSE_TOO_NEW, message=v.encode())
        return None, ws

    # --- The panel's socket --------------------------------------------------

    async def panel_socket(self, req: web.Request):
        got, refusal = await self.gate(req)
        if refusal is not None:
            return refusal
        ws, rec, dev = got
        try:
            first = await ws.receive(timeout=10)
            hello = odc.loads(first.data.encode()) if first.type == WSMsgType.TEXT else {}
        except (odc.Bad, asyncio.TimeoutError):
            hello = {}
        secret = odc.unhex(hello.get("secret"), 32) if hello.get("t") == "hello" else None
        if secret is None or odc.sha256(secret) != rec.secret_hash:
            self.refuse("panel-secret")
            await ws.close(code=odc.CLOSE_REFUSED)
            return ws
        old, rec.panel, rec.sess, rec.fw = rec.panel, ws, hello.get("sess"), str(hello.get("fw", ""))
        if old is not None and not old.closed:
            await old.close(code=1000, message=b"replaced")
        await self.to_ctls(rec, {"t": "panel", "online": True, "sess": rec.sess})
        if rec.ctls:
            await ws.send_str(odc.dumps({"t": "watch", "on": True}))
        async for msg in ws:
            if msg.type == WSMsgType.TEXT:
                await self.from_panel(rec, dev, ws, msg.data)
            elif msg.type == WSMsgType.BINARY:
                pass  # The panel sends nothing binary in v1.
        if rec.panel is ws:
            rec.panel = None
            await self.to_ctls(rec, {"t": "panel", "online": False})
        return ws

    async def from_panel(self, rec: Record, dev: bytes, ws, text: str):
        if len(text.encode()) > odc.MAX_TEXT_OUT:
            return
        try:
            m = odc.loads(text.encode())
        except odc.Bad:
            return
        t = m.get("t")
        if t == "keys":
            rec.keys = {}
            for e in m.get("keys", [])[: odc.MAX_KEYS]:
                pub = odc.unhex(e.get("pub"), 32)
                if pub is not None and odc.is_int(e.get("f")):
                    rec.keys[odc.key_id(pub)] = (pub, e["f"])
            # A removed key's sockets go.
            for c, k in list(rec.ctls.items()):
                if k not in rec.keys:
                    await c.close(code=odc.CLOSE_REFUSED)
        elif t == "info":
            rec.name = str(m.get("name", ""))[:64]
        elif t == "pair-open":
            pid = odc.unhex(m.get("pair_id"), 10)
            ttl = m.get("ttl")
            if pid and odc.is_int(ttl):
                exp = time.monotonic() + min(ttl, odc.PAIR_TTL_S)
                rec.pairs[pid] = exp
                self.finds[pid] = (dev, exp)
        elif t == "pair-close":
            pid = odc.unhex(m.get("pair_id"), 10)
            rec.pairs.pop(pid, None)
            self.finds.pop(pid, None)
        elif t == "ack":
            k = odc.unhex(m.get("k"), 8)
            for c, ck in rec.ctls.items():
                if ck == k:
                    await c.send_str(text)
        elif t == "credit":
            c = rec.transfers.get(m.get("id"))
            if c is not None and not c.closed:
                await c.send_str(text)
        elif t == "status":
            await self.to_ctls(rec, m)
        elif t == "paired":
            g = rec.guests.get(m.get("g"))
            if g is not None:
                await g[0].send_str(text)
                if "k" in m:
                    rec.pairs.pop(g[1], None)
                    self.finds.pop(g[1], None)
        elif t == "forget":
            self.panels.pop(dev, None)
            await ws.close(code=1000)

    async def to_ctls(self, rec: Record, m: dict):
        s = odc.dumps(m)
        for c in list(rec.ctls):
            if not c.closed:
                await c.send_str(s)

    async def watch(self, rec: Record):
        if rec.panel is not None and not rec.panel.closed:
            await rec.panel.send_str(odc.dumps({"t": "watch", "on": bool(rec.ctls)}))

    # --- Controllers and guests -------------------------------------------------

    async def ctl_socket(self, req: web.Request):
        origin = req.headers.get("Origin")
        if origin is not None and origin != ORIGIN:
            self.refuse("origin")
            return web.Response(status=403)
        got, refusal = await self.gate(req)
        if refusal is not None:
            return refusal
        ws, rec, dev = got
        nonce = os.urandom(16)
        await ws.send_str(odc.dumps({"t": "challenge", "nonce": nonce.hex()}))
        try:
            first = await ws.receive(timeout=10)
            m = odc.loads(first.data.encode()) if first.type == WSMsgType.TEXT else {}
        except (odc.Bad, asyncio.TimeoutError):
            m = {}
        if m.get("t") == "attach":
            return await self.controller(ws, rec, dev, nonce, m)
        if m.get("t") == "guest":
            return await self.guest(ws, rec, dev, m)
        self.refuse("no-attach")
        await ws.close(code=odc.CLOSE_REFUSED)
        return ws

    async def controller(self, ws, rec: Record, dev: bytes, nonce: bytes, m: dict):
        k = odc.unhex(m.get("k"), 8)
        sig = odc.unhex(m.get("sig"), 64)
        entry = rec.keys.get(k) if k else None
        if entry is None or sig is None or not odc.verify_attach(entry[0], dev, nonce, sig):
            # The same close for an unknown key and a bad signature.
            self.refuse("attach")
            await ws.close(code=odc.CLOSE_REFUSED)
            return ws
        online = rec.panel is not None and not rec.panel.closed
        rec.ctls[ws] = k
        await ws.send_str(odc.dumps({"t": "attached", "sess": rec.sess if online else None, "online": online, "name": rec.name, "fw": rec.fw}))
        if len(rec.ctls) == 1:
            await self.watch(rec)
        try:
            async for msg in ws:
                if msg.type == WSMsgType.TEXT:
                    await self.ctl_text(ws, rec, k, msg.data)
                elif msg.type == WSMsgType.BINARY:
                    await self.ctl_binary(ws, rec, k, msg.data)
        finally:
            rec.ctls.pop(ws, None)
            for tid in [t for t, c in rec.transfers.items() if c is ws]:
                rec.transfers.pop(tid)
            if not rec.ctls:
                await self.watch(rec)
        return ws

    async def ctl_text(self, ws, rec: Record, k: bytes, text: str):
        if text == "ping":
            await ws.send_str("pong")
            return
        try:
            m = odc.loads(text.encode())
        except odc.Bad:
            return
        if m.get("t") == "invite-open":
            iid = odc.unhex(m.get("invite_id"), 10)
            if iid:
                rec.invites[iid] = (ws, time.monotonic() + odc.PAIR_TTL_S)
        elif m.get("t") == "invite-close":
            rec.invites.pop(odc.unhex(m.get("invite_id"), 10), None)
        elif m.get("t") == "add-ok":
            g = rec.guests.get(m.get("g"))
            if g is not None and g[2] is ws:
                await g[0].send_str(text)

    async def ctl_binary(self, ws, rec: Record, k: bytes, frame: bytes):
        def ack(code, fk=None):
            return ws.send_str(odc.dumps({"t": "ack", "k": (fk or k).hex(), "err": code}))

        if frame[:1] == bytes([odc.TYPE_CHUNK]):
            try:
                tid, _off, _data = odc.parse_chunk(frame)
            except odc.Bad:
                return
            # Forwarded only for the transfer this socket started (the
            # credit window itself is the panel's to enforce in v1's mock).
            if rec.transfers.get(tid) is ws and rec.panel is not None:
                await rec.panel.send_bytes(frame)
            else:
                self.refuse("chunk")
            return
        try:
            fk = odc.precheck({kk: v[0] for kk, v in rec.keys.items()}, frame)
        except odc.Refusal as r:
            self.refuse(r.code)
            if r.k is not None:
                await ack(r.code, r.k)
            return
        if fk != k:
            # A frame signed by another paired key, sent on this socket.
            self.refuse("not-this-socket")
            await ack("unknown-key", fk)
            return
        bucket = rec.rates.setdefault(k, odc.Bucket(odc.RATE_PER_KEY, now_us()))
        t = now_us()
        if not bucket.take(t) or not rec.panel_rate.take(t):
            self.refuse("rate")
            await ack("rate")
            return
        if rec.panel is None or rec.panel.closed:
            await ack("busy")
            return
        # The server reads a `send` in passing to route its credits.
        try:
            body = odc.loads(frame[odc.COMMAND_HEAD:])
            if body.get("c") == "send" and odc.is_int(body.get("a", {}).get("id")):
                rec.transfers[body["a"]["id"]] = ws
        except (odc.Bad, AttributeError):
            pass
        await rec.panel.send_bytes(frame)

    async def guest(self, ws, rec: Record, dev: bytes, m: dict):
        pid = odc.unhex(m.get("pair_id"), 10)
        iid = odc.unhex(m.get("invite_id"), 10)
        now = time.monotonic()
        inviter = None
        if pid and rec.pairs.get(pid, 0) > now:
            wid = pid
        elif iid and iid in rec.invites and rec.invites[iid][1] > now:
            wid, inviter = iid, rec.invites[iid][0]
        else:
            self.refuse("guest-no-window")
            await ws.close(code=odc.CLOSE_REFUSED)
            return ws
        if len(rec.guests) >= 2:
            await ws.close(code=odc.CLOSE_STAY_AWAY, message=b"1")
            return ws
        g = rec.next_g
        rec.next_g = rec.next_g % 250 + 1
        rec.guests[g] = (ws, wid, inviter)
        await ws.send_str(odc.dumps({"t": "guest-ok", "g": g}))
        try:
            async with asyncio.timeout(GUEST_LIMIT_S):
                async for msg in ws:
                    if msg.type != WSMsgType.TEXT or len(msg.data.encode()) > odc.MAX_TEXT_IN:
                        continue
                    try:
                        gm = odc.loads(msg.data.encode())
                    except odc.Bad:
                        continue
                    gm["g"] = g
                    if self.rewrite_label is not None and "label" in gm:
                        gm["label"] = self.rewrite_label
                    if gm.get("t") == "pair" and inviter is None and rec.panel is not None:
                        await rec.panel.send_str(odc.dumps(gm))
                    elif gm.get("t") == "add-req" and inviter is not None and not inviter.closed:
                        await inviter.send_str(odc.dumps(gm))
        except TimeoutError:
            await ws.close(code=1000)
        finally:
            rec.guests.pop(g, None)
        return ws


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--listen", default="127.0.0.1:8787")
    a = ap.parse_args()
    host, port = a.listen.rsplit(":", 1)
    web.run_app(Server().app(), host=host, port=int(port), print=None)


if __name__ == "__main__":
    main()
