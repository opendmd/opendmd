# /// script
# requires-python = ">=3.11"
# dependencies = ["pynacl==1.5.0", "aiohttp==3.12.15"]
# ///
"""Connect v1 end to end on localhost: the mock server, the mock panel and
controllers, all in one process (no device, no Cloudflare, no network past
127.0.0.1). Throwaway keys made at run time.

    uv run tools/connect/roundtrip.py          # or: make connect-mock

Blue: register, hello, pair by the LAN link's secret, attach, commands
with acks and status, add another device (invite, confirm code,
add-controller), a GIF send (credit, a taken name, resume after the
panel reconnects, nothing charged twice), remove, the panel reconnecting. Red, each refused: a
replay, a tampered or unsigned frame, a stale session, names outside the
allow-list, bad arguments, an oversize body, a flood, a stranger's attach,
a made-up id, a guest with no window, a wrong pairing MAC three times, a
reused window, a wrong panel secret, a foreign Origin, a version from the
future, a one-level key adding another, a removed key. One `PASS` line per
case; `ROUNDTRIP PASS` at the end.
"""
from __future__ import annotations

import asyncio
import os
import sys

import aiohttp
import nacl.signing
from aiohttp import web

import odc
from mock_panel import Panel
from mock_server import Server

passed = 0


def ok(cond, what):
    global passed
    if not cond:
        print(f"FAIL: {what}")
        sys.exit(1)
    passed += 1
    print(f"PASS {what}")


class Controller:
    def __init__(self, base: str, dev: bytes, label: str):
        self.base, self.dev, self.label = base, dev, label
        self.sk = nacl.signing.SigningKey(os.urandom(32))
        self.pub = bytes(self.sk.verify_key)
        self.k = odc.key_id(self.pub)
        self.n = 0
        self.sess: bytes | None = None
        self.ws = None
        self.inbox: list = []
        self.last = 0.0

    def protocols(self, version="odc1"):
        return (version, "d." + odc.dev_text(self.dev))

    async def connect(self, http, version="odc1", headers=None):
        ws = await http.ws_connect(f"{self.base}/v1/ctl", protocols=self.protocols(version), headers=headers or {}, autoping=True)
        self.ws, self.inbox = ws, []
        return ws

    async def attach(self, http):
        ws = await self.connect(http)
        ch = await self.until("challenge")
        if ch is None:
            return None
        nonce = bytes.fromhex(ch["nonce"])
        await ws.send_str(odc.dumps({"t": "attach", "k": self.k.hex(), "sig": odc.attach_signature(self.sk, self.dev, nonce).hex()}))
        at = await self.until("attached")
        if at and at.get("sess"):
            self.sess = bytes.fromhex(at["sess"])
        return at

    async def recv(self, timeout=3.0):
        try:
            msg = await self.ws.receive(timeout=timeout)
        except asyncio.TimeoutError:
            return None
        if msg.type == aiohttp.WSMsgType.TEXT:
            if msg.data == "pong":
                return {"t": "pong"}
            m = odc.loads(msg.data.encode())
            if m.get("t") == "panel" and m.get("sess"):
                self.sess = bytes.fromhex(m["sess"])
            return m
        return {"t": "_closed", "code": self.ws.close_code}

    async def until(self, t, pred=lambda m: True, timeout=3.0):
        for i, m in enumerate(self.inbox):
            if m.get("t") == t and pred(m):
                return self.inbox.pop(i)
        loop = asyncio.get_running_loop()
        end = loop.time() + timeout
        while (left := end - loop.time()) > 0:
            m = await self.recv(left)
            if m is None:
                return None
            if m.get("t") == "_closed":
                self.inbox.append(m)
                return m if t == "_closed" else None
            if m.get("t") == t and pred(m):
                return m
            self.inbox.append(m)
        return None

    def frame(self, c, a=None, n=None, sess=None, dev=None):
        if n is None:
            self.n += 1
            n = self.n
        return odc.sign_command(self.sk, odc.command_body(dev or self.dev, sess or self.sess, n, c, a))

    async def send(self, frame):
        """Send a frame at the 4 a second a controller keeps to."""
        loop = asyncio.get_running_loop()
        wait = self.last + 0.26 - loop.time()
        if wait > 0:
            await asyncio.sleep(wait)
        self.last = loop.time()
        await self.ws.send_bytes(frame)

    async def command(self, c, a=None, **kw):
        f = self.frame(c, a, **kw)
        await self.send(f)
        return await self.ack(f)

    async def ack(self, frame):
        n = odc.loads(frame[odc.COMMAND_HEAD:]).get("n") if len(frame) > odc.COMMAND_HEAD else None
        return await self.until("ack", lambda m: m.get("n", n) == n)

    async def closed(self, timeout=3.0):
        m = await self.until("_closed", timeout=timeout)
        return m["code"] if m else None


def send_args(tid: int, name: str, gif: bytes, copy: bytes = b"", play: bool = False) -> dict:
    a = {"id": tid, "name": name, "size": len(gif), "sha256": odc.sha256(gif).hex(), "play": play}
    if copy:
        a["copy_size"], a["copy_sha256"] = len(copy), odc.sha256(copy).hex()
    return a


async def stream(ctl, tid: int, data: bytes, start: int = 0, stop: int | None = None, upto: int | None = None, timeout: float = 5.0):
    """Send `data` from `start` inside the panel's credits until its last
    credit (`end`) or `stop` bytes; the credits seen and the end message."""
    at, credits = start, []
    while True:
        while upto is not None and at < min(upto, stop if stop is not None else len(data)):
            n = min(odc.MAX_CHUNK, upto - at)
            await ctl.ws.send_bytes(odc.chunk_frame(tid, at, data[at:at + n]))
            at += n
        if stop is not None and at >= stop:
            return credits, None, at
        m = await ctl.until("credit", lambda m: m.get("id") == tid, timeout=timeout)
        if m is None:
            return credits, None, at
        credits.append(m)
        if "end" in m:
            return credits, m, at
        upto = m["upto"]


async def guest_pair(http, base, dev, secret, ctl: Controller, mac=None):
    """A new browser pairing as a guest with the secret from the link."""
    async with http.post(f"{base}/v1/find", data=odc.dumps({"pair_id": odc.pair_id(secret).hex()})) as r:
        if r.status != 200:
            return {"find": r.status}
        found = odc.parse_dev((await r.json())["dev"])
    ws = await ctl.connect(http)
    await ctl.until("challenge")
    await ws.send_str(odc.dumps({"t": "guest", "pair_id": odc.pair_id(secret).hex()}))
    g = await ctl.until("guest-ok")
    if g is None:
        return {"closed": await ctl.closed()}
    replies = []
    for m in mac if isinstance(mac, list) else [mac]:
        m = m or odc.pair_mac(secret, found, ctl.pub, ctl.label.encode())
        await ws.send_str(odc.dumps({"t": "pair", "pub": ctl.pub.hex(), "label": ctl.label, "mac": m.hex()}))
        replies.append(await ctl.until("paired"))
    await ws.close()
    return {"dev": found, "replies": replies}


async def main():
    server = Server()
    runner = web.AppRunner(server.app())
    await runner.setup()
    site = web.TCPSite(runner, "127.0.0.1", 0)
    await site.start()
    port = site._server.sockets[0].getsockname()[1]
    base = f"http://127.0.0.1:{port}"

    async with aiohttp.ClientSession() as http:
        async with http.get(f"{base}/v1/health") as r:
            h = await r.json()
        ok(h == {"ok": True, "v": 1, "min_v": 1}, "health answers {ok, v, min_v}")
        async with http.get(f"{base}/v1/nothing") as r:
            ok(r.status == 404, "an unknown route is 404")

        panel = Panel(base)
        task = asyncio.create_task(panel.run(http))
        await asyncio.wait_for(panel.connected.wait(), 5)
        dev = panel.dev
        ok(dev is not None and odc.dev_tag_ok(dev, server.id_key), "panel registered with a tagged id and said hello")

        # --- Pairing by the LAN link's secret ------------------------------------
        a = Controller(base, dev, "A phone")
        secret = await panel.open_pairing()
        await asyncio.sleep(0.05)
        res = await guest_pair(http, base, dev, secret, a)
        rep = res["replies"][0]
        ok(res["dev"] == dev and rep and rep.get("k") == a.k.hex(), "guest found the panel by pair_id and paired")
        ok(rep and bytes.fromhex(rep["mac2"]) == odc.paired_mac(secret, dev, a.pub), "the panel's mac2 proves it saw the secret")
        ok(panel.keys[a.k].flags == odc.FLAG_CAN_ADD, "a key paired on the panel may add others")
        res = await guest_pair(http, base, dev, secret, Controller(base, dev, "late"))
        ok(res.get("find") == 404, "red: a used pairing window is gone (find 404)")

        # Wrong MAC three times on a fresh window.
        secret2 = await panel.open_pairing()
        await asyncio.sleep(0.05)
        x = Controller(base, dev, "x")
        res = await guest_pair(http, base, dev, secret2, x, mac=[bytes(32)] * 3 + [None])
        errs = [r.get("err") if r else None for r in res["replies"]]
        ok(errs == ["bad-mac", "bad-mac", "bad-mac", "closed"], f"red: a wrong pairing MAC three times closes the window ({errs})")
        ok(x.k not in panel.keys, "red: ...and that key is not on the list")

        # A key swapped by the server: the guest's MAC with another key.
        secret3 = await panel.open_pairing()
        await asyncio.sleep(0.05)
        y, swapped = Controller(base, dev, "y"), Controller(base, dev, "swapped")
        res = await guest_pair(http, base, dev, secret3, swapped, mac=[odc.pair_mac(secret3, dev, y.pub, swapped.label.encode())])
        ok(res["replies"][0].get("err") == "bad-mac" and swapped.k not in panel.keys, "red: a key swapped in transit fails the MAC")

        # A hostile relay rewriting the label in a relayed `pair`.
        secret4 = await panel.open_pairing()
        await asyncio.sleep(0.05)
        server.rewrite_label = "Owner (admin)"
        z = Controller(base, dev, "z phone")
        res = await guest_pair(http, base, dev, secret4, z)
        server.rewrite_label = None
        ok(res["replies"][0].get("err") == "bad-mac" and z.k not in panel.keys, "red: a label rewritten by the server fails the pairing MAC")

        # --- Attach and commands ---------------------------------------------------
        at = await a.attach(http)
        ok(at and at["online"] and a.sess == bytes.fromhex(panel.session.sess.hex()), "attach: challenge, signed attach, attached with the panel's sess")
        st = await a.until("status")
        ok(st is not None and panel.watched, "watch on: the panel sends status while a controller looks")
        r = await a.command("next")
        ok(r and r.get("ok") is True, "next: ack ok")
        r = await a.command("brightness", {"v": 100})
        st = await a.until("status", lambda m: m.get("brightness") == 100)
        ok(r and r.get("ok") and st is not None, "brightness 100: ack ok, status follows")
        r = await a.command("pause")
        ok(r and r.get("r") == {"paused": True}, "pause: ack carries r.paused")
        r = await a.command("play", {"path": "@formula:fire"})
        ok(r and r.get("ok") and panel.path == "@formula:fire", "play a bundled Formula")
        r = await a.command("playlists")
        ok(r and isinstance(r.get("r"), list), "playlists: a listing in r")
        r = await a.command("name", {"name": " Den​ panel "})
        ok(r and r.get("ok") and panel.name == "Den panel", "name: stored cleaned of invisible characters")
        await a.ws.send_str("ping")
        ok(await a.until("pong") is not None, "text ping answered")

        # --- Red over the wire -------------------------------------------------------
        f = a.frame("next")
        await a.send(f)
        await a.ack(f)
        await a.send(f)
        r = await a.ack(f)
        ok(r and r.get("err") == "replay", "red: a replayed frame is refused (replay)")
        r = await a.command("next", n=1)
        ok(r and r.get("err") == "replay", "red: an old counter is refused (replay)")
        for c in ("update", "delete", "wifi", "reboot", "firmware", "config"):
            r = await a.command(c)
            ok(r and r.get("err") == "not-allowed", f"red: '{c}' is not in the allow-list")
        before = len(panel.log)
        f = bytearray(a.frame("brightness", {"v": 10}))
        f[-3] ^= 1
        await a.send(bytes(f))
        r = await a.until("ack", lambda m: m.get("err") == "bad-sig")
        ok(r is not None and len(panel.log) == before, "red: a tampered frame is stopped by the server (bad-sig), the panel never sees it")
        f = bytearray(a.frame("next"))
        f[9:73] = bytes(64)
        await a.send(bytes(f))
        r = await a.until("ack", lambda m: m.get("err") == "bad-sig")
        ok(r is not None, "red: an unsigned frame is refused (bad-sig)")
        r = await a.command("next", sess=bytes(12))
        ok(r and r.get("err") == "stale-session", "red: a frame for another session is refused (stale-session)")
        r = await a.command("next", dev=odc.mint_dev(bytes(10), server.id_key))
        ok(r and r.get("err") == "stale-session", "red: a frame for another panel is refused")
        r = await a.command("brightness", {"v": 129})
        ok(r and r.get("err") == "bad-arg", "red: brightness over the ceiling (bad-arg)")
        for p in ("/gifs/../wifi.json", "/cache/a/xx/b.gif", "@formula:~sent", "@formula?psb=AAAA"):
            r = await a.command("play", {"path": p})
            ok(r and r.get("err") == "bad-arg", f"red: play {p} (bad-arg)")
        f = a.frame("play", {"path": "/gifs/" + "x" * 700 + ".gif"})
        await a.send(f)
        r = await a.until("ack", lambda m: m.get("err") == "too-big")
        ok(r is not None, "red: a body over 700 bytes (too-big)")
        await asyncio.sleep(1.1)
        frames = [a.frame("next") for _ in range(30)]
        for f in frames:
            await a.ws.send_bytes(f)
        acks = []
        while len(acks) < len(frames) and (m := await a.until("ack", timeout=3)) is not None:
            acks.append(m)
        n_ok = sum(1 for x in acks if x and x.get("ok"))
        n_rate = sum(1 for x in acks if x and x.get("err") == "rate")
        ok(n_ok <= 6 and n_rate >= 24, f"red: a flood of 30 is rate-limited ({n_ok} ok, {n_rate} rate)")
        await asyncio.sleep(1.1)

        stranger = Controller(base, dev, "stranger")
        await stranger.attach(http)
        ok(await stranger.closed() == odc.CLOSE_REFUSED, "red: a stranger's attach is closed 4403")
        imposter = Controller(base, dev, "imposter")
        imposter.k = a.k
        await imposter.attach(http)
        ok(await imposter.closed() == odc.CLOSE_REFUSED, "red: a paired key's id with another key's signature: 4403 alike")
        made_up = Controller(base, odc.mint_dev(os.urandom(10), b"not the server's key"), "m")
        try:
            await made_up.connect(http)
            ok(False, "a made-up id was upgraded")
        except aiohttp.WSServerHandshakeError as e:
            ok(e.status == 404, "red: a made-up id is refused before any object wakes (404)")
        g = Controller(base, dev, "g")
        ws = await g.connect(http)
        await g.until("challenge")
        await ws.send_str(odc.dumps({"t": "guest", "pair_id": bytes(10).hex()}))
        ok(await g.closed() == odc.CLOSE_REFUSED, "red: a guest with no open window is closed 4403")
        try:
            await a.connect(http, headers={"Origin": "https://evil.example"})
            ok(False, "a foreign Origin was upgraded")
        except aiohttp.WSServerHandshakeError as e:
            ok(e.status == 403, "red: a foreign Origin is refused (403)")
        future = Controller(base, dev, "future")
        await future.connect(http, version="odc2")
        ok(await future.closed() == odc.CLOSE_TOO_NEW, "red: only a version from the future offered: closed 4406")
        async with http.post(f"{base}/v1/register", data='{"v":1,"secret_hash":"00"}') as r:
            ok(r.status == 400, "red: a malformed registration is refused")

        # --- Add another device ----------------------------------------------------
        await a.ws.close()
        await a.attach(http)
        inv = odc.b32(os.urandom(16), 128)
        await a.ws.send_str(odc.dumps({"t": "invite-open", "invite_id": odc.invite_id(inv).hex(), "ttl": 300}))
        await asyncio.sleep(0.05)
        server.rewrite_label = "Owner (admin)"
        z = Controller(base, dev, "z laptop")
        zws = await z.connect(http)
        await z.until("challenge")
        await zws.send_str(odc.dumps({"t": "guest", "invite_id": odc.invite_id(inv).hex()}))
        await z.until("guest-ok")
        await zws.send_str(odc.dumps({"t": "add-req", "pub": z.pub.hex(), "label": z.label, "mac": odc.add_mac(inv, dev, z.pub, z.label.encode()).hex()}))
        zreq = await a.until("add-req")
        server.rewrite_label = None
        await zws.close()
        ok(zreq and zreq["label"] == "Owner (admin)" and odc.add_mac(inv, dev, z.pub, zreq["label"].encode()) != bytes.fromhex(zreq["mac"]), "red: a label rewritten in a relayed add-req fails the inviter's MAC check")
        await asyncio.sleep(0.05)
        b = Controller(base, dev, "B laptop")
        bws = await b.connect(http)
        await b.until("challenge")
        await bws.send_str(odc.dumps({"t": "guest", "invite_id": odc.invite_id(inv).hex()}))
        gok = await b.until("guest-ok")
        await bws.send_str(odc.dumps({"t": "add-req", "pub": b.pub.hex(), "label": b.label, "mac": odc.add_mac(inv, dev, b.pub, b.label.encode()).hex()}))
        req = await a.until("add-req")
        good = req and odc.add_mac(inv, dev, bytes.fromhex(req["pub"]), req["label"].encode()) == bytes.fromhex(req["mac"])
        ok(gok and good, "add another device: the inviter checks the guest's MAC")
        ok(odc.confirm_code(bytes.fromhex(req["pub"])) == odc.confirm_code(b.pub), "both screens show the same 4-character code")
        r = await a.command("add-controller", {"pub": req["pub"], "label": req["label"]})
        ok(r and r.get("ok") and panel.keys[b.k].flags == 0 and panel.keys[b.k].added_by == a.k, "add-controller: B added, one level only")
        await a.ws.send_str(odc.dumps({"t": "add-ok", "g": req["g"], "k": b.k.hex()}))
        ok(await b.until("add-ok") is not None, "the guest hears it was added")
        await bws.close()
        at = await b.attach(http)
        ok(at and at["online"], "B attaches with its own key")
        c_pub = bytes(nacl.signing.SigningKey(os.urandom(32)).verify_key)
        r = await b.command("add-controller", {"pub": c_pub.hex(), "label": "C"})
        ok(r and r.get("err") == "not-allowed", "red: a one-level key cannot add another (not-allowed)")
        r = await b.command("controllers")
        ok(r and len(r.get("r", [])) == 2, "controllers: both listed")

        # --- Send a GIF --------------------------------------------------------------
        ok(server.caps.get("send") is True and server.caps.get("win") == panel.win, "hello's caps: send, win")
        gif = b"GIF89a" + os.urandom(5000)
        copy = b"GIF89a" + os.urandom(2100)
        args = send_args(41, "sent.gif", gif, copy, play=True)
        f = a.frame("send", args)
        await a.send(f)
        r = await a.ack(f)
        ok(r and r.get("ok") and r.get("r") == {"offset": 0}, "send: ack ok, from offset 0")
        credits, end, _ = await stream(a, 41, gif + copy)
        ok(end and end.get("end") == "ok" and end.get("path") == "/gifs/Sent/sent.gif" and panel.landed["sent.gif"] == (gif, copy), "send: landed byte for byte (GIF and copy), the last credit says ok")
        ok(all(c["k"] == a.k.hex() for c in credits) and not server.refused.get("credit-beyond-win"), f"send: every credit for its key, none past caps.win ({len(credits)} credits)")
        ok(panel.path == "/gifs/Sent/sent.gif" and panel.quota_used == len(gif) + len(copy), "send: played, size + copy_size charged")
        f = a.frame("send", send_args(42, "sent.gif", gif))
        await a.send(f)
        await a.ack(f)
        _, end, _ = await stream(a, 42, gif)
        ok(end and end.get("path") == "/gifs/Sent/sent_2.gif", "send: a taken name gets the next one, never an overwrite")
        # Red: a chunk past the credit is dropped by the server.
        big = b"GIF89a" + os.urandom(9000)
        f = a.frame("send", send_args(43, "big.gif", big))
        await a.send(f)
        await a.ack(f)
        c0 = await a.until("credit", lambda m: m.get("id") == 43)
        before = server.refused.get("chunk", 0)
        await a.ws.send_bytes(odc.chunk_frame(43, c0["upto"], big[c0["upto"]:c0["upto"] + 100]))
        await asyncio.sleep(0.1)
        ok(server.refused.get("chunk", 0) == before + 1 and panel.xfer and panel.xfer["off"] == 0, "red: a chunk past the credit is dropped by the server, the panel unmoved")
        # Red: another key naming the held transfer: busy.
        f = b.frame("send", send_args(43, "big.gif", big))
        await b.send(f)
        r = await b.ack(f)
        ok(r and r.get("err") == "busy", "red: another key's send of a held id: busy")
        # A reconnect midway: the same key signs it again and resumes.
        _, _, at = await stream(a, 43, big, upto=c0["upto"], stop=4096)
        await asyncio.sleep(0.1)
        await panel.ws.close()
        await task
        task = asyncio.create_task(panel.run(http))
        await asyncio.wait_for(panel.connected.wait(), 5)
        await a.until("panel", lambda m: m.get("online") is True)
        used = panel.quota_used
        # (Credits from before the reconnect are stale.)
        a.inbox = [m for m in a.inbox if m.get("t") != "credit"]
        f = a.frame("send", send_args(43, "big.gif", big))
        await a.send(f)
        r = await a.ack(f)
        off = (r or {}).get("r", {}).get("offset")
        ok(r and r.get("ok") and r["r"].get("resumed") and off == panel.xfer["off"] and off > 0, f"resume after a reconnect: from {off}")
        _, end, _ = await stream(a, 43, big, start=off)
        ok(end and end.get("end") == "ok" and panel.landed["big.gif"][0] == big and panel.quota_used == used, "resumed send landed whole, nothing charged again")
        # Red: a wrong hash: refused at the end.
        bad = dict(send_args(44, "bad.gif", gif), sha256="00" * 32)
        f = a.frame("send", bad)
        await a.send(f)
        await a.ack(f)
        _, end, _ = await stream(a, 44, gif)
        ok(end and end.get("end") == "bad-arg" and "bad.gif" not in panel.landed, "red: bytes that miss the signed hash: end bad-arg, nothing landed")
        # Red: over the quota.
        panel.quota_used = odc.QUOTA_BYTES - 10
        f = a.frame("send", send_args(45, "q.gif", gif))
        await a.send(f)
        r = await a.ack(f)
        ok(r and r.get("err") == "quota", "red: over the panel's quota: quota")

        # --- Reconnect, remove -------------------------------------------------------
        old = b.frame("next")
        await panel.ws.close()
        await task
        task = asyncio.create_task(panel.run(http))
        await asyncio.wait_for(panel.connected.wait(), 5)
        ok(await b.until("panel", lambda m: m.get("online") is True) is not None, "the panel reconnects; controllers hear the new sess")
        await b.send(old)
        r = await b.ack(old)
        ok(r and r.get("err") == "stale-session", "red: a frame signed before the reconnect is void (stale-session)")
        r = await b.command("next")
        ok(r and r.get("ok"), "signed again with the new sess, it goes")
        r = await b.command("remove-controller", {"k": a.k.hex()})
        ok(r and r.get("ok") and a.k not in panel.keys, "any paired key may remove another")
        ok(await a.closed() == odc.CLOSE_REFUSED, "the removed key's socket is closed")
        await a.attach(http)
        ok(await a.closed() == odc.CLOSE_REFUSED, "red: a removed key cannot attach again")

        # Another panel claiming this id without its secret.
        fake = Panel(base)
        fake.dev = dev
        fake_task = asyncio.create_task(fake.run(http))
        await fake_task
        ok(fake.closed_with == odc.CLOSE_REFUSED and panel.ws is not None and not panel.ws.closed, "red: a wrong panel secret is closed 4403, the real panel stays")

        # Forget: the record goes.
        await panel.send({"t": "forget"})
        await task
        try:
            await b.connect(http)
            ok(False, "a forgotten panel's id still upgrades")
        except aiohttp.WSServerHandshakeError as e:
            ok(e.status == 404, "forget: the server drops the record")

    await runner.cleanup()
    print(f"ROUNDTRIP PASS ({passed} checks)")


if __name__ == "__main__":
    asyncio.run(main())
