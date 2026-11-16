# /// script
# requires-python = ">=3.11"
# dependencies = ["pynacl==1.5.0", "aiohttp==3.12.15"]
# ///
"""Remote control's client in the emulator (`make emu-connect`): the real
firmware against tools/connect/mock_server.py, over TLS. Correctness only,
never timing (every figure is the emulator's).

The image (`make emu-image-connect`: `emu-census` + `connect`) is built
for the host `connect.test` at 10.0.2.2:8443, a local test root pinned
beside the real ones, a 30 s hold after boot, and one test controller key
(made from a fixed string here: `pubkey` prints it) it starts with. It
boots with WiFi credentials seeded, on esp32sim's virtual access point,
inside a private network namespace (`unshare -rn`: nothing reaches a real
network or device); 10.0.2.2 is put on the loopback, where the mock server
listens with TLS 1.2 (the panel's side) and on 127.0.0.1:8787 in plain
HTTP (the controllers' side, this script's).

Blue: Remote control off by default; switched on over the LAN (no hold);
registration, `hello`, `keys`, `info`; a controller attaches, watches and
gets status; play, next, previous, pause, brightness, playlist,
playlists, controllers, name, add-controller; the LAN routes; the panel
reconnecting after a server close; a refusal (4403) and a too-new close
(4406) handled by policy; the server gone and back; switch off; forget
(a new identity); a restart (the identity kept, back after the hold).
Red, each refused by the panel itself (frames put straight onto the
panel's socket, past the server's pre-check) and none reaching the player:
replayed, stale-session, wrong-panel, tampered, unsigned-by-a-stranger,
disallowed (update, delete, wifi, reboot, ...), bad paths and arguments,
an over-long body, a one-level key adding another, a removed key, a flood
(the panel's own rate), chunks and unknown frame types (no answer).

    uv run tools/connect/emu_lane.py --image <full.bin> --certs <dir> [--card sd.img]
    uv run tools/connect/emu_lane.py pubkey        # the image's test key
    uv run tools/connect/emu_lane.py certs DIR     # the test root and chain
"""
from __future__ import annotations

import argparse
import asyncio
import hashlib
import json
import os
import re
import shutil
import ssl
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
sys.path.insert(0, str(REPO / "tools" / "emu"))
sys.path.insert(0, str(HERE))

import nacl.signing  # noqa: E402

import odc  # noqa: E402

HOST = "connect.test"
PORT = 8080
CTL_PORT = 8787
# Throwaway: a test key made from a fixed string, for emulator images only.
TEST_SEED = hashlib.sha256(b"opendmd emu-connect test controller, never a real key").digest()
SCRATCH = os.environ.get("SCRATCH")
EMU_DIR = Path(os.environ.get("EMU_DIR") or (Path(SCRATCH) / "opendmd-emu-sim" if SCRATCH else REPO / "target" / "emu-sim"))
FAIL = [
    (re.compile(r"Guru Meditation"), "Guru Meditation"),
    (re.compile(r"panicked at|\bpanic\b.*\bat\b|abort\(\) was called|\bassert failed\b", re.I), "panic/abort"),
    (re.compile(r"stack overflow", re.I), "stack overflow"),
    (re.compile(r"^Backtrace:"), "backtrace"),
    (re.compile(r"CORRUPT HEAP|heap_caps_.*(alloc|free) failed.*abort", re.I), "heap corruption"),
]
ACCEPTED = re.compile(r"connect: command n=(\d+) from [0-9a-f]+: (ok|refused)")
REFUSED = re.compile(r"connect: command refused: (\S+)")


def test_key() -> nacl.signing.SigningKey:
    return nacl.signing.SigningKey(TEST_SEED)


def make_certs(d: Path, name: str = HOST):
    """A P-384 root, a P-384 intermediate and a P-256 leaf for `name` (the
    shape of Cloudflare's chains), valid from now for 30 days."""
    d.mkdir(parents=True, exist_ok=True)

    def run(*a):
        subprocess.run(["openssl", *a], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    run("ecparam", "-name", "secp384r1", "-genkey", "-noout", "-out", str(d / "root.key"))
    run("req", "-x509", "-new", "-key", str(d / "root.key"), "-sha384", "-days", "30", "-subj", "/O=OpenDMD test/CN=Connect emulator test root",
        "-addext", "basicConstraints=critical,CA:TRUE", "-addext", "keyUsage=critical,keyCertSign,cRLSign", "-out", str(d / "root.pem"))
    run("ecparam", "-name", "secp384r1", "-genkey", "-noout", "-out", str(d / "int.key"))
    run("req", "-new", "-key", str(d / "int.key"), "-subj", "/O=OpenDMD test/CN=Connect emulator test intermediate", "-out", str(d / "int.csr"))
    (d / "int.ext").write_text("basicConstraints=critical,CA:TRUE,pathlen:0\nkeyUsage=critical,keyCertSign,cRLSign\n")
    run("x509", "-req", "-in", str(d / "int.csr"), "-CA", str(d / "root.pem"), "-CAkey", str(d / "root.key"), "-CAcreateserial",
        "-days", "29", "-sha384", "-extfile", str(d / "int.ext"), "-out", str(d / "int.pem"))
    run("ecparam", "-name", "prime256v1", "-genkey", "-noout", "-out", str(d / "leaf.key"))
    run("req", "-new", "-key", str(d / "leaf.key"), "-subj", f"/CN={name}", "-out", str(d / "leaf.csr"))
    (d / "leaf.ext").write_text(f"subjectAltName=DNS:{name}\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\n")
    run("x509", "-req", "-in", str(d / "leaf.csr"), "-CA", str(d / "int.pem"), "-CAkey", str(d / "int.key"), "-CAcreateserial",
        "-days", "28", "-sha384", "-extfile", str(d / "leaf.ext"), "-out", str(d / "leaf.pem"))
    (d / "chain.pem").write_text((d / "leaf.pem").read_text() + (d / "int.pem").read_text())
    run("x509", "-in", str(d / "root.pem"), "-outform", "der", "-out", str(d / "root.der"))
    for f in d.glob("*.csr"):
        f.unlink()


class Fail(Exception):
    pass


def lane_server():
    """The mock server, recording what the panel says (every text it sends,
    acks included, whoever they are for)."""
    from mock_server import Server

    class Recording(Server):
        def __init__(self):
            super().__init__()
            self.said: list[dict] = []
            self.registered = 0
            self.hellos = 0

        async def register(self, req):
            r = await super().register(req)
            if r.status == 200:
                self.registered += 1
            return r

        async def from_panel(self, rec, dev, ws, text):
            try:
                self.said.append(odc.loads(text.encode()))
            except odc.Bad:
                self.said.append({"t": "_bad", "text": text[:80]})
            await super().from_panel(rec, dev, ws, text)

    return Recording()


class Lane:
    def __init__(self, emu, srv, logs: Path):
        self.emu, self.srv, self.logs = emu, srv, logs
        self.base = f"http://127.0.0.1:{PORT}"
        self.ctl_base = f"http://127.0.0.1:{CTL_PORT}"
        self.passes: list[str] = []
        self.report: dict = {}

    # --- the panel's LAN API --------------------------------------------------
    def _http(self, method, path, body=None, timeout=30.0):
        data = None if body is None else json.dumps(body).encode()
        headers = {"Content-Type": "application/json"} if body is not None else {}
        if method != "GET" and data is None:
            data = b""
        req = urllib.request.Request(self.base + path, method=method, data=data, headers=headers)
        for k in range(10):
            try:
                with urllib.request.urlopen(req, timeout=timeout) as r:
                    return r.status, json.loads(r.read() or b"null")
            except urllib.error.HTTPError as e:
                if e.code == 503 and k < 9:
                    time.sleep(2)
                    continue
                return e.code, None
            except (urllib.error.URLError, TimeoutError, ConnectionError):
                self.emu.check()
                if k == 9:
                    raise
                time.sleep(2)
        return None, None

    async def http(self, method, path, body=None):
        return await asyncio.to_thread(self._http, method, path, body)

    async def status(self):
        return (await self.http("GET", "/api/status"))[1] or {}

    async def connect_info(self):
        return (await self.http("GET", "/api/connect"))[1] or {}

    def ok(self, name, detail=""):
        line = f"pass  {name}: {detail}"
        self.passes.append(line)
        print(line, flush=True)

    def expect(self, cond, name, detail=""):
        if not cond:
            raise Fail(f"{name}: {detail}")
        self.ok(name, detail)

    async def wait(self, what, cond, timeout, every=2.0):
        end = time.time() + timeout
        last = None
        while time.time() < end:
            self.emu.check()
            if self.emu.done:
                raise Fail(f"{what}: the emulator stopped ({self.emu.tail(3)})")
            try:
                last = await cond()
            except Exception as e:  # noqa: BLE001
                last = e
            if last and not isinstance(last, Exception):
                return last
            await asyncio.sleep(every)
        c = (await self.status()).get("connect") or {}
        raise Fail(f"{what}: not within {timeout:.0f} s wall (state {c.get('state')!r}, last_error {c.get('last_error')!r}; last {last!r})")

    def rec(self):
        return next(iter(self.srv.panels.values()), None)

    def panel_ws(self):
        r = self.rec()
        return r.panel if r and r.panel is not None and not r.panel.closed else None

    async def open_session(self, what, before_sess=None, timeout=900):
        """Wait for the panel's socket (a session other than `before_sess`)."""

        async def up():
            r = self.rec()
            return r and r.panel is not None and not r.panel.closed and r.sess and r.sess != before_sess and r.sess
        return await self.wait(what, up, timeout)

    def said(self, t, start=0, pred=lambda m: True):
        return [m for m in self.srv.said[start:] if m.get("t") == t and pred(m)]

    async def panel_says(self, what, t, start, pred=lambda m: True, timeout=120):
        async def got():
            hits = self.said(t, start, pred)
            return hits[0] if hits else None
        return await self.wait(what, got, timeout, every=0.5)

    def accepted_since(self, mark):
        return [m for _, m in self.emu.find(ACCEPTED, mark) if m[2] == "ok"]

    # --- frames straight onto the panel's socket (past the server's checks) -----------
    async def inject(self, frame: bytes):
        ws = self.panel_ws()
        if ws is None:
            raise Fail("no panel socket to put a frame on")
        await ws.send_bytes(frame)

    async def injected_ack(self, what, frame, k: bytes, timeout=120):
        start = len(self.srv.said)
        n = None
        if len(frame) > odc.COMMAND_HEAD:
            try:
                n = odc.loads(frame[odc.COMMAND_HEAD:]).get("n")
            except odc.Bad:
                n = None
        await self.inject(frame)
        return await self.panel_says(what, "ack", start, lambda m: m.get("k") == k.hex() and (n is None or m.get("n", n) == n), timeout)

    # --- the steps -------------------------------------------------------------------
    async def run(self):
        import aiohttp
        from roundtrip import Controller

        st = await self.wait("the API answers", self.status, 900, every=5)
        self.expect("connect" in (st.get("features") or []), "feature", f"features {st.get('features')}")
        info = await self.connect_info()
        self.expect(info.get("enabled") is False and info.get("registered") is False, "off by default, not registered", json.dumps({k: info.get(k) for k in ("enabled", "registered", "state")}))
        await asyncio.sleep(10)
        self.expect(self.srv.registered == 0, "off: nothing sent to the server", f"{self.srv.registered} registrations")
        heap_off = st.get("heap") or {}

        # Switched on over the LAN: at once (the hold is for boots).
        code, info = await self.http("PUT", "/api/connect", {"enabled": True})
        self.expect(code == 200 and info.get("enabled") is True, "PUT /api/connect on", f"{code} {info and info.get('state')}")
        code, _ = await self.http("PUT", "/api/connect", {"enabled": "yes"})
        self.expect(code == 400, "PUT /api/connect with a bad body: 400", f"{code}")
        sess = await self.open_session("registered and open (the clock floor first: 60 emulated s)")
        rec = self.rec()
        dev = next(iter(self.srv.panels))
        self.expect(self.srv.registered == 1 and odc.dev_tag_ok(dev, self.srv.id_key), "registered once, a tagged id", odc.dev_text(dev))
        key = test_key()
        tk = odc.key_id(bytes(key.verify_key))
        self.expect(tk in rec.keys and rec.keys[tk][1] == odc.FLAG_CAN_ADD, "keys after hello: the image's test key, can_add", f"{[k.hex() for k in rec.keys]}")
        hello_fw = rec.fw
        self.expect(hello_fw and rec.name, "hello's fw and info's name", f"fw {hello_fw!r}, name {rec.name!r}")
        info = await self.connect_info()
        self.expect(info.get("registered") is True and info.get("state") == "open" and "secret" not in json.dumps(info), "GET /api/connect: open, registered, no secret",
                    json.dumps({k: info.get(k) for k in ("state", "registered", "controllers")}))
        st = await self.status()
        c = st.get("connect") or {}
        self.expect(c.get("state") == "open" and c.get("opened", 0) >= 1, "/api/status connect", json.dumps(c))
        self.report["heap"] = {"off": heap_off, "open": st.get("heap")}

        async with aiohttp.ClientSession() as http:
            a = Controller(self.ctl_base, dev, "emulator")
            a.sk, a.pub, a.k = key, bytes(key.verify_key), tk
            at = await a.attach(http)
            self.expect(at and at.get("online") is True and a.sess and bytes.fromhex(sess) == a.sess, "a controller attaches: online, the panel's sess", json.dumps(at))
            stat = await a.until("status", timeout=120)
            self.expect(stat and "now_playing" in stat and stat.get("id") is not None and "heap" not in stat, "watched: status comes", json.dumps(stat)[:300])
            self.report["status_bytes"] = len(json.dumps(stat))

            async def cmd(c, a_=None, ctl=None, timeout=120):
                ctl = ctl or a
                f = ctl.frame(c, a_)
                await ctl.send(f)
                n = odc.loads(f[odc.COMMAND_HEAD:])["n"]
                return await ctl.until("ack", lambda m: m.get("n") == n, timeout=timeout)

            # --- blue: the command set -------------------------------------------
            p0 = (await self.status()).get("paused")
            r = await cmd("pause")
            self.expect(r and r.get("ok") and (r.get("r") or {}).get("paused") is (not p0), "pause: ack with paused", json.dumps(r))
            self.expect((await self.status()).get("paused") is (not p0), "pause: the player paused", "")
            r = await cmd("pause")
            self.expect(r and r.get("ok") and (await self.status()).get("paused") is p0, "pause again: resumed", json.dumps(r))
            o0 = ((await self.status()).get("now_playing") or {}).get("ordinal")
            r = await cmd("next")
            self.expect(r and r.get("ok"), "next: ack ok", json.dumps(r))
            await self.wait("next took effect", lambda: self._moved(o0), 120)
            self.ok("next: the player moved on")
            r = await cmd("prev")
            self.expect(r and (r.get("ok") or r.get("err") in ("bad-arg", "busy")), "prev: answered", json.dumps(r))
            r = await cmd("brightness", {"v": 40})
            cfg = (await self.http("GET", "/api/config"))[1] or {}
            self.expect(r and r.get("ok") and cfg.get("brightness") == 40, "brightness 40: ack ok, /api/config says 40", f"{r} {cfg.get('brightness')}")
            mx = cfg.get("max_brightness", 255)
            r = await cmd("brightness", {"v": mx + 1})
            self.expect(r and r.get("err") == "bad-arg" and ((await self.http("GET", "/api/config"))[1] or {}).get("brightness") == 40,
                        "red: brightness over the ceiling: bad-arg, unchanged", json.dumps(r))
            path = ((await self.status()).get("now_playing") or {}).get("next") or ((await self.status()).get("now_playing") or {}).get("path")
            if path and path.startswith("/gifs/"):
                r = await cmd("play", {"path": path})
                self.expect(r and r.get("ok"), f"play {path}: ack ok", json.dumps(r))
                await self.wait("the play took effect", lambda: self._playing(path), 120)
                self.ok("play: on screen")
            r = await cmd("play", {"path": "@formula:starfield"})
            self.expect(r and r.get("ok"), "play a bundled Formula: ack ok", json.dumps(r))
            await self.wait("the Formula on screen", lambda: self._playing("@formula:starfield"), 120)
            self.ok("play a Formula: on screen")
            r = await cmd("play", {"path": "/gifs/no such folder/none.gif"})
            self.expect(r and r.get("err") == "bad-arg", "play a GIF that is not there: bad-arg", json.dumps(r))
            r = await cmd("playlists")
            self.expect(r and r.get("ok") and isinstance((r.get("r") or {}).get("sets"), list), "playlists: a bounded list", json.dumps(r)[:300])
            r = await cmd("playlist", {"set": ""})
            self.expect(r and r.get("ok"), "playlist \"\" (the main list): ok", json.dumps(r))
            r = await cmd("playlist", {"set": "no-such-set"})
            if r and r.get("r", {}).get("pending"):
                await asyncio.sleep(5)
            self.expect(r and (r.get("err") == "bad-arg" or (r.get("r") or {}).get("pending")), "playlist that is not there: refused", json.dumps(r))
            await asyncio.sleep(3)
            cfg = (await self.http("GET", "/api/config"))[1] or {}
            self.expect(cfg.get("set") in ("", None), "...and the playlist unchanged", f"set {cfg.get('set')!r}")
            r = await cmd("controllers")
            ctls = (r or {}).get("r") or []
            self.expect(r and r.get("ok") and any(x.get("k") == tk.hex() and x.get("last_s") is not None for x in ctls), "controllers: the list with last use", json.dumps(r)[:300])
            mark = len(self.srv.said)
            r = await cmd("name", {"name": "Den ​ panel"})
            info_msg = await self.panel_says("info after a rename", "info", mark)
            self.expect(r and r.get("ok") and info_msg.get("name", "").startswith("Den") and "\u200b" not in info_msg.get("name", ""), "name: stored, cleaned (no zero width), info sent", json.dumps(info_msg))
            self.expect((await self.connect_info()).get("name", "").startswith("Den"), "GET /api/connect: the name", "")

            # One-level keys: A adds B (no can_add); B cannot add.
            b = Controller(self.ctl_base, dev, "second")
            mark = len(self.srv.said)
            r = await cmd("add-controller", {"pub": b.pub.hex(), "label": "second"})
            self.expect(r and r.get("ok") and (r.get("r") or {}).get("k") == b.k.hex(), "add-controller: ok", json.dumps(r))
            await self.panel_says("keys after the add", "keys", mark)
            await asyncio.sleep(0.5)
            self.expect(b.k in rec.keys and rec.keys[b.k][1] == 0, "the server has B, without can_add", "")
            at = await b.attach(http)
            self.expect(at and at.get("online"), "B attaches", "")
            r = await cmd("pause", ctl=b)
            self.expect(r and r.get("ok"), "B commands: pause ok", json.dumps(r))
            await cmd("pause", ctl=b)
            cc = Controller(self.ctl_base, dev, "third")
            r = await cmd("add-controller", {"pub": cc.pub.hex(), "label": "third"}, ctl=b)
            self.expect(r and r.get("err") == "not-allowed" and cc.k not in rec.keys, "red: a one-level key adding another: not-allowed", json.dumps(r))

            # --- red: frames straight onto the panel's socket ------------------------
            mark_line = self.emu.mark()
            pz = (await self.status()).get("paused")
            stranger = nacl.signing.SigningKey(os.urandom(32))
            sk_ = odc.key_id(bytes(stranger.verify_key))
            last = a.frame("pause")
            await a.send(last)
            await a.ack(last)
            pz = (await self.status()).get("paused")
            mark_line = self.emu.mark()
            cases = [
                ("replayed", last, a.k, "replay"),
                ("stale session", odc.sign_command(key, odc.command_body(dev, os.urandom(12), 10**9, "pause")), a.k, "stale-session"),
                ("another panel's", odc.sign_command(key, odc.command_body(os.urandom(16), a.sess, 10**9 + 1, "pause")), a.k, "stale-session"),
                ("tampered", last[:-5] + bytes([last[-5] ^ 1]) + last[-4:], a.k, "bad-sig"),
                ("signed by a stranger", odc.sign_command(stranger, odc.command_body(dev, a.sess, 1, "pause")), sk_, "unknown-key"),
                ("a stranger's key id, A's signature", bytes([1]) + sk_ + last[9:], sk_, "unknown-key"),
                ("over-long body", odc.sign_command(key, odc.command_body(dev, a.sess, 10**9 + 2, "pause", {"x": "y" * 700})), a.k, "too-big"),
                ("unsigned (zero signature)", bytes([1]) + a.k + bytes(64) + odc.command_body(dev, a.sess, 10**9 + 3, "pause"), a.k, "bad-sig"),
            ]
            n = 10**9 + 10
            for c in ("update", "firmware", "delete", "wifi", "reboot", "restart", "config", "upload", "forget", "PAUSE", "send"):
                n += 1
                cases.append((f"not on the allow-list: {c}", odc.sign_command(key, odc.command_body(dev, a.sess, n, c, {"id": 1, "name": "x.gif", "size": 1, "sha256": "00" * 32} if c == "send" else None)), a.k, "not-allowed"))
            for bad in ("/gifs/../wifi.txt", "/cache/a/xx/b.gif", "@formula:~sent", "@formula?psb=AAAA", "/filelist.txt", "/gifs/.hidden/x.gif", "@card:boot"):
                n += 1
                cases.append((f"bad path {bad}", odc.sign_command(key, odc.command_body(dev, a.sess, n, "play", {"path": bad})), a.k, "bad-arg"))
            for name, frame, k, want in cases:
                got = await self.injected_ack(f"an answer to the {name} frame", frame, k)
                self.expect(got.get("err") == want, f"red: {name}: {want}", json.dumps(got))
                await asyncio.sleep(1.0)  # under the panel's own rates (4 a second a key), in emulated time too
            for name, frame in (("a chunk with no transfer", bytes([2]) + (1).to_bytes(4, "big") + bytes(4) + b"x" * 10),
                                ("an unknown frame type", bytes([7]) + b"x" * 80), ("a short command frame", bytes([1]) + b"x" * 20)):
                start = len(self.srv.said)
                await self.inject(frame)
                await asyncio.sleep(5)
                self.expect(not self.said("ack", start), f"red: {name}: dropped, no answer", "")
            # A flood: 40 at once, the panel's own rate refuses past it.
            start = len(self.srv.said)
            fl = []
            for _ in range(40):
                n += 1
                fl.append(odc.sign_command(key, odc.command_body(dev, a.sess, n, "controllers")))
            for f in fl:
                await self.inject(f)
            await self.wait("the flood answered", lambda: self._said_n("ack", start, 40), 600, every=2)
            acks = self.said("ack", start)
            rate = sum(1 for m in acks if m.get("err") == "rate")
            self.expect(rate >= 20 and all(m.get("ok") or m.get("err") == "rate" for m in acks), "red: a flood of 40: the panel's rate refuses most", f"{rate} rate, {len(acks) - rate} ok")
            st = await self.status()
            self.expect(st.get("paused") is pz, "red: none of the refused frames reached the player (pause unchanged)", f"paused {st.get('paused')}")
            bad_accepts = [m for _, m in self.emu.find(ACCEPTED, mark_line) if m[2] == "ok"]
            self.expect(len(bad_accepts) <= 8 + 4, "red: only the flood's rate-allowed 'controllers' were carried out", f"{len(bad_accepts)} accepted lines")
            self.report["refusals"] = sorted({m[1] for _, m in self.emu.find(REFUSED, mark_line)})

            # --- the LAN routes: a controller removed ---------------------------
            await asyncio.sleep(2)
            mark = len(self.srv.said)
            code, info = await self.http("DELETE", f"/api/connect/controllers/{b.k.hex()}")
            self.expect(code == 200 and not any(x.get("k") == b.k.hex() for x in info.get("controllers", [])), "DELETE /api/connect/controllers/<B>", f"{code}")
            await self.panel_says("keys after the LAN removal", "keys", mark)
            closed = await b.closed(timeout=30)
            # (The mock closes it 4403; a client that was not reading may
            # see the end as 1006. What matters: the server dropped B.)
            self.expect(closed is not None and b.k not in self.rec().ctls.values(), "the server closed B's socket", f"close code {closed}")
            got = await self.injected_ack("B's frame after its removal", odc.sign_command(b.sk, odc.command_body(dev, a.sess, 99, "pause")), b.k)
            self.expect(got.get("err") == "unknown-key", "red: a removed key: unknown-key", json.dumps(got))
            code, _ = await self.http("DELETE", f"/api/connect/controllers/{b.k.hex()}")
            self.expect(code == 404, "DELETE an unknown controller: 404", f"{code}")

            # --- reconnects and the server's closes ----------------------------
            old = a.sess
            await self.panel_ws().close(code=1001, message=b"bye")
            new = await self.open_session("back after a server close", before_sess=rec.sess, timeout=900)
            self.expect(new != old.hex(), "reconnected after a server close: a new sess", new)
            await asyncio.sleep(3)
            got = await self.injected_ack("an old session's frame", odc.sign_command(key, odc.command_body(dev, old, 10**12, "pause")), a.k)
            self.expect(got.get("err") == "stale-session", "red: a frame signed for the session before: stale-session", json.dumps(got))

            await self.panel_ws().close(code=odc.CLOSE_TOO_NEW, message=b"1")
            s406 = await self.open_session("back after a too-new close (transient: the usual backoff)", before_sess=rec.sess, timeout=900)
            info = await self.connect_info()
            self.expect(info.get("too_old") is False, "4406: transient, no 'update needed'", f"state {info.get('state')}")

            await self.panel_ws().close(code=odc.CLOSE_REFUSED)
            await self.wait("refused", lambda: self._info(lambda i: i.get("refused") is True), 300)
            st = await self.status()
            self.ok("4403: refused, Settings told to set up again", f"state {(st.get('connect') or {}).get('state')!r}")
            await asyncio.sleep(20)
            self.expect(self.panel_ws() is None, "4403: no attempt on the normal schedule", "")
            await self.http("PUT", "/api/connect", {"enabled": True})
            await self.open_session("back at once after a switch-on", before_sess=s406, timeout=600)
            self.ok("a switch-on in Settings: at once, past the refusal's wait")

            # The server gone, then back.
            site = self.srv.tls_site
            await site.stop()
            ws = self.panel_ws()
            if ws is not None:
                await ws.close(code=1001)
            await self.wait("the panel notices the server gone", lambda: self._info(lambda i: i.get("state") != "open"), 600)
            await asyncio.sleep(30)
            st = await self.status()
            self.ok("server off: backing off", f"{(st.get('connect') or {}).get('state')!r}, {(st.get('connect') or {}).get('last_error')!r}")
            await self.srv.start_tls()
            await self.open_session("back once the server is", before_sess=rec.sess, timeout=1200)
            self.ok("server back: reconnected by itself")

            # The server lost the record (a registration that never said
            # hello is deleted): 404 at the upgrade, a new id, the
            # controllers kept.
            regs = self.srv.registered
            gone = self.srv.panels.pop(dev)
            if gone.panel is not None and not gone.panel.closed:
                await gone.panel.close(code=1001)
            await self.wait("registered again after a 404", lambda: self._dev_changed(dev), 900)
            new_dev = next(iter(self.srv.panels))
            await self.open_session("open under the id made again", timeout=600)
            self.expect(self.srv.registered == regs + 1 and tk in self.rec().keys, "404 at the upgrade: registered again, the controllers kept", odc.dev_text(new_dev))
            info = await self.connect_info()
            self.expect(info.get("refused") is False, "404: not a refusal", f"state {info.get('state')!r}")
            dev = new_dev

            # Switch off.
            code, info = await self.http("PUT", "/api/connect", {"enabled": False})
            await self.wait("switched off", lambda: self._info(lambda i: i.get("state") == "off" and self.panel_ws() is None), 300)
            self.ok("PUT /api/connect off: closed, off")
            st_off = await self.status()
            await self.http("PUT", "/api/connect", {"enabled": True})
            await self.open_session("on again", before_sess=None, timeout=600)
            await asyncio.sleep(20)
            st_on = await self.status()
            self.report["heap_idle"] = {"off": st_off.get("heap"), "on_idle": st_on.get("heap")}

            # Forget: the old identity says forget; a new one is made.
            regs = self.srv.registered
            code, info = await self.http("POST", "/api/connect/forget")
            self.expect(code == 200, "POST /api/connect/forget", f"{code}")
            await self.wait("a new identity", lambda: self._dev_changed(dev), 900)
            new_dev = next(iter(self.srv.panels))
            self.expect(dev not in self.srv.panels and self.srv.registered == regs + 1, "forget: the old record deleted, a new id registered", odc.dev_text(new_dev))
            dev = new_dev
            await self.open_session("open under the new identity", timeout=600)

        # A planned restart: the identity is in NVS, back by itself after the hold.
        regs = self.srv.registered
        self.emu.resets_allowed += 1
        try:
            await self.http("POST", "/api/reboot")
        except Exception:  # noqa: BLE001
            pass
        await self.wait("back after the restart", lambda: self._uptime_below(300), 900, every=3)
        before = self.rec().sess
        await self.open_session("open by itself after the restart", before_sess=before, timeout=900)
        st = await self.status()
        self.expect(self.srv.registered == regs and dev in self.srv.panels, "after a restart: the same identity, no new registration", f"uptime {st.get('uptime_secs')}")
        self.expect(st.get("uptime_secs", 0) >= 30, "after a restart: not before the hold", f"open at {st.get('uptime_secs')} s")
        st = await self.status()
        self.expect(not self.emu.failure and self.emu.resets <= self.emu.resets_allowed, "no panic, no unplanned restart", f"resets {self.emu.resets}")

    async def _moved(self, o0):
        return ((await self.status()).get("now_playing") or {}).get("ordinal") != o0

    async def _playing(self, path):
        return ((await self.status()).get("now_playing") or {}).get("path") == path

    async def _info(self, f):
        return f(await self.connect_info())

    async def _said_n(self, t, start, n):
        return len(self.said(t, start)) >= n

    async def _dev_changed(self, dev):
        return bool(self.srv.panels) and dev not in self.srv.panels and self.rec() and self.rec().panel is not None

    async def _uptime_below(self, s):
        st = await self.status()
        return st.get("uptime_secs", 9999) < s and "connect" in st


async def serve_and_run(emu, logs, certs: Path) -> Lane:
    from aiohttp import web

    srv = lane_server()
    runner = web.AppRunner(srv.app())
    await runner.setup()
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.minimum_version = ssl.TLSVersion.TLSv1_2
    ctx.maximum_version = ssl.TLSVersion.TLSv1_2
    ctx.load_cert_chain(certs / "chain.pem", certs / "leaf.key")

    async def start_tls():
        srv.tls_site = web.TCPSite(runner, "10.0.2.2", 8443, ssl_context=ctx)
        await srv.tls_site.start()

    srv.start_tls = start_tls
    await start_tls()
    await web.TCPSite(runner, "127.0.0.1", CTL_PORT).start()
    lane = Lane(emu, srv, logs)
    try:
        await lane.run()
    finally:
        await runner.cleanup()
    return lane


def main():
    if len(sys.argv) > 1 and sys.argv[1] == "pubkey":
        print(bytes(test_key().verify_key).hex())
        return
    if len(sys.argv) > 2 and sys.argv[1] == "certs":
        make_certs(Path(sys.argv[2]))
        print(f"certs in {sys.argv[2]}: root.der (the image's test root), chain.pem + leaf.key (the server's)")
        return
    import api_lane

    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--image", type=Path, required=True)
    ap.add_argument("--certs", type=Path, required=True)
    ap.add_argument("--card", type=Path, default=EMU_DIR / "card" / "sd.img")
    ap.add_argument("--rom", type=Path)
    ap.add_argument("--seconds", type=int, default=6000, help="emulated seconds at most")
    ap.add_argument("--wall", type=int, default=10800, help="wall-clock limit (s)")
    a = ap.parse_args()
    sim = EMU_DIR / "src" / "target" / "release" / "esp32sim"
    rom = a.rom or next(iter(sorted(Path.home().glob(".espressif/tools/esp-rom-elfs/*/esp32_rev300_rom.elf"))[::-1]), None)
    for what, p in (("emulator (make emu-build)", sim), ("image", a.image), ("card (make emu-card)", a.card), ("rom", rom), ("certs", a.certs / "chain.pem")):
        if not p or not Path(p).exists():
            sys.exit(f"EMU_CONNECT_FAIL: missing {what}: {p}")
    seeded = api_lane.seeded_image(a.image, EMU_DIR / "connect" / "seed")
    if os.environ.get("CX_NETNS") != "1":
        if not shutil.which("unshare"):
            sys.exit("EMU_CONNECT_FAIL: no `unshare`: the lane runs in its own network namespace")
        os.execvpe("unshare", ["unshare", "--user", "--map-root-user", "--net", "--", sys.executable, __file__, *sys.argv[1:]], {**os.environ, "CX_NETNS": "1"})
    ip = shutil.which("ip") or next((p for p in ("/usr/sbin/ip", "/sbin/ip") if os.path.exists(p)), None)
    if not ip or subprocess.run([ip, "link", "set", "lo", "up"]).returncode != 0 or subprocess.run([ip, "addr", "add", "10.0.2.2/32", "dev", "lo"]).returncode != 0:
        sys.exit("EMU_CONNECT_FAIL: cannot set up the namespace's loopback (lo up, 10.0.2.2)")
    logs = EMU_DIR / "logs" / f"connect-{time.strftime('%Y%m%d-%H%M%S')}"
    logs.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ)
    cmd = [*api_lane.machine(sim, "esp32", seeded, rom, ["--sd", str(a.card)]),
           "--max-seconds", str(a.seconds), "--console", "uart0",
           "--wifi", api_lane.WIFI, "--net", "nat", "--hostfwd", f"tcp:127.0.0.1:{PORT}-:80", "--no-dump"]
    emu = api_lane.Emulator(cmd, env, logs / "console.log", FAIL, "connect")
    emu.start()
    timer = threading.Timer(a.wall, emu.kill)
    timer.daemon = True
    timer.start()
    t0 = time.time()
    code = 0
    lane = None
    try:
        lane = asyncio.run(serve_and_run(emu, logs, a.certs))
        print(f"EMU_CONNECT_OK ({len(lane.passes)} checks, {time.time() - t0:.0f} s wall)", flush=True)
    except (Fail, api_lane.LaneFail, Exception) as e:  # noqa: BLE001
        print(f"EMU_CONNECT_FAIL: {e}\n--- console tail:\n{emu.tail(25)}", flush=True)
        code = 1
    finally:
        if lane is not None:
            (logs / "report.json").write_text(json.dumps({"passes": lane.passes, **lane.report}, indent=1) + "\n")
        print(f"console {logs / 'console.log'}", flush=True)
        emu.kill()
    sys.exit(code)


if __name__ == "__main__":
    main()
