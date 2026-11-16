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
(4406) handled by policy; the server gone and back; pairing (the LAN
link's window with its QR code on the panel, a guest found by `pair_id`,
the key stored with can_add, `keys` before `paired`, `mac2` over the
label as sent, `pair-close`, "Remote control added", the new device's
commands; a window cancelled on the LAN); switch off; forget (a new
identity); a restart (the identity kept, back after the hold).
Red, each refused by the panel itself (frames put straight onto the
panel's socket, past the server's pre-check) and none reaching the player:
replayed, stale-session, wrong-panel, tampered, unsigned-by-a-stranger,
disallowed (update, delete, wifi, reboot, ...), bad paths and arguments,
an over-long body, a one-level key adding another, a removed key, a flood
(the panel's own rate), chunks and unknown frame types (no answer).
Pairing's red: the pair route while off (409) and without the API key
when one is set (401, whatever the body); three wrong MACs (closed, and
`pair-close` at once); a window used once (the server forgets it; a
`pair` put straight onto the panel's socket is `closed`); a label
rewritten by the relay (`bad-mac`, the window kept for the right one); a
window left to expire (the image's 90 s: `pair-close`, the QR code down,
a late `pair` refused).

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
# The image's pairing window (Makefile EMU_CONNECT_PAIR_TTL_S).
PAIR_TTL_S = int(os.environ.get("EMU_CONNECT_PAIR_TTL_S", "90"))
SCREEN = re.compile(r"remote control screen: (pairing QR code|added|off)")
LANE_KEY = "emu-lane-api-key-0123"
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
    def _http(self, method, path, body=None, timeout=30.0, key=None):
        data = None if body is None else json.dumps(body).encode()
        headers = {"Content-Type": "application/json"} if body is not None else {}
        if key:
            headers["Authorization"] = "Bearer " + key
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

    async def http(self, method, path, body=None, key=None):
        return await asyncio.to_thread(self._http, method, path, body, 30.0, key)

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
        code, _ = await self.http("POST", "/api/connect/pair")
        self.expect(code == 409, "red: POST /api/connect/pair while off: 409, no window", f"{code}")

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
            # Config commands: one a second (a drag's extras are refused).
            await asyncio.sleep(1.5)
            f1, f2 = a.frame("brightness", {"v": 42}), a.frame("brightness", {"v": 43})
            await a.ws.send_bytes(f1)
            await a.ws.send_bytes(f2)
            r1, r2 = await a.ack(f1), await a.ack(f2)
            self.expect(r1 and r1.get("ok") and r2 and r2.get("err") == "rate", "red: two config commands within the second: the second is rate", f"{r1} {r2}")
            # A LAN change racing a remote one: both stay (the remote one
            # carries only its field, never a copy of all the settings).
            await asyncio.sleep(1.5)
            cfg = (await self.http("GET", "/api/config"))[1] or {}
            dwell = (cfg.get("dwell_max") or 10) + 1
            f = a.frame("brightness", {"v": 60})
            await a.ws.send_bytes(f)
            code, _ = await self.http("PUT", "/api/config", {"dwell_max": dwell})
            r = await a.ack(f)
            await asyncio.sleep(7)  # past the remote save's 5 s
            cfg = (await self.http("GET", "/api/config"))[1] or {}
            self.expect(code == 200 and r and r.get("ok") and cfg.get("brightness") == 60 and cfg.get("dwell_max") == dwell,
                        "a LAN config change racing a remote brightness: both kept", f"PUT {code}, ack {r}, brightness {cfg.get('brightness')}, dwell_max {cfg.get('dwell_max')} (want {dwell})")
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
            await asyncio.sleep(1.2)
            r = await cmd("playlist", {"set": ""})
            self.expect(r and r.get("ok"), "playlist \"\" (the main list): ok", json.dumps(r))
            await asyncio.sleep(1.2)
            r = await cmd("playlist", {"set": "no-such-set"})
            if r and r.get("r", {}).get("pending"):
                await asyncio.sleep(5)
            self.expect(r and (r.get("err") == "bad-arg" or (r.get("r") or {}).get("pending")), "playlist that is not there: refused", json.dumps(r))
            await asyncio.sleep(3)
            cfg = (await self.http("GET", "/api/config"))[1] or {}
            self.expect(cfg.get("set") in ("", None), "...and the playlist unchanged", f"set {cfg.get('set')!r}")
            # A real playlist on the card, racing a LAN field: both stay,
            # and the player is on it.
            sets = ((await cmd("playlists")) or {}).get("r", {}).get("sets") or []
            self.expect(bool(sets), "the card has a playlist to switch to", f"{sets}")
            await asyncio.sleep(1.2)
            dwell = (((await self.http("GET", "/api/config"))[1] or {}).get("dwell_max") or 10) + 1
            f = a.frame("playlist", {"set": sets[0]})
            await a.ws.send_bytes(f)
            code, _ = await self.http("PUT", "/api/config", {"dwell_max": dwell})
            r = await a.ack(f)
            await self.wait("on the playlist", lambda: self._manifest(sets[0]), 300)
            cfg = (await self.http("GET", "/api/config"))[1] or {}
            self.expect(code == 200 and r and (r.get("ok")) and cfg.get("set") == sets[0] and cfg.get("dwell_max") == dwell,
                        "a remote playlist racing a LAN field: both kept, the player on it", f"ack {r}, set {cfg.get('set')!r}, dwell_max {cfg.get('dwell_max')} (want {dwell})")
            await asyncio.sleep(1.2)
            r = await cmd("playlist", {"set": ""})
            await self.wait("back on the main list", lambda: self._manifest("filelist"), 300)
            self.ok("back to the main list", json.dumps(r))
            await asyncio.sleep(1.2)
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
            su = ((await self.status()).get("connect") or {}).get("stack_unused")
            self.expect(isinstance(su, int) and su >= 1024, "the network worker's stack after the command set (card present): >= 1 KB left", f"{su} B")
            self.report["stack_unused"] = su

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
            # Config buckets over time: 14 controllers, each added, sending a
            # brightness and removed (with A, 15 buckets; the core's own
            # counters cap a session at 16 keys, B is one); after the
            # reconnect below two more keys' config commands are still ok
            # (17 in all: buckets pruned with their keys, cleared per session).
            async def churn(label):
                c_ = Controller(self.ctl_base, dev, label)
                r = await cmd("add-controller", {"pub": c_.pub.hex(), "label": label})
                if not (r and r.get("ok")):
                    raise Fail(f"churn: add {label}: {r}")
                await asyncio.sleep(1.2)
                sess_now = bytes.fromhex(self.rec().sess)
                got = await self.injected_ack(f"{label}'s brightness", odc.sign_command(c_.sk, odc.command_body(dev, sess_now, 1, "brightness", {"v": 30})), c_.k)
                code, _ = await self.http("DELETE", f"/api/connect/controllers/{c_.k.hex()}")
                return got, code
            # (A's counter past the red cases' injected ones, this session.)
            a.n = max(a.n, 10**12 + 100)
            outs = []
            for i in range(14):
                got, code = await churn(f"churn{i}")
                outs.append((got.get("ok") or got.get("err"), code))
            self.expect(all(o == (True, 200) for o in outs), "14 controllers added, a config command each, removed", f"{outs}")
            old = a.sess
            await self.panel_ws().close(code=1001, message=b"bye")
            new = await self.open_session("back after a server close", before_sess=rec.sess, timeout=900)
            self.expect(new != old.hex(), "reconnected after a server close: a new sess", new)
            await asyncio.sleep(3)
            got = await self.injected_ack("an old session's frame", odc.sign_command(key, odc.command_body(dev, old, 10**12, "pause")), a.k)
            self.expect(got.get("err") == "stale-session", "red: a frame signed for the session before: stale-session", json.dumps(got))
            await a.ws.close()
            await a.attach(http)
            later = [await churn(f"later{i}") for i in range(2)]
            self.expect(all(g.get("ok") is True and c == 200 for g, c in later), "keys 16 and 17's config commands, after a reconnect: ok (buckets pruned, per session)", f"{later}")

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

            await self.pairing(http, dev, a, tk)

            # The server lost the record. With controllers: a refusal like
            # 403, retried every 15 minutes, nothing erased (the owner may
            # choose Forget).
            d = Controller(self.ctl_base, dev, "kept")
            # A fresh attach: this session's sess (several reconnects ago
            # the controller's copy went stale).
            await a.ws.close()
            await a.attach(http)
            await asyncio.sleep(1)
            r = await cmd("add-controller", {"pub": d.pub.hex(), "label": "kept"})
            self.expect(r and r.get("ok"), "a second controller before the id is lost", json.dumps(r))
            await asyncio.sleep(2)
            regs = self.srv.registered
            gone = self.srv.panels.pop(dev)
            if gone.panel is not None and not gone.panel.closed:
                await gone.panel.close(code=1001)
            await self.wait("refused after a 404 with controllers", lambda: self._info(lambda i: i.get("refused") is True), 900)
            info = await self.connect_info()
            ks = sorted(x.get("k") for x in info.get("controllers", []))
            await asyncio.sleep(20)
            self.expect(self.srv.registered == regs and ks == sorted([tk.hex(), d.k.hex()]) and not self.srv.panels,
                        "404 with controllers: refused, nothing erased, no new registration", f"controllers {ks}, state {info.get('state')!r}")
            # With none (the legitimate case: a registration that never said
            # hello is deleted): a new id by itself.
            for k in (d.k, tk):
                code, _ = await self.http("DELETE", f"/api/connect/controllers/{k.hex()}")
                self.expect(code == 200, f"DELETE controller {k.hex()[:6]} on the LAN", f"{code}")
            await self.http("PUT", "/api/connect", {"enabled": True})
            await self.wait("registered again after a 404 with no controllers", lambda: self._new_record(), 900)
            new_dev = next(iter(self.srv.panels))
            await self.open_session("open under the new id", timeout=600)
            info = await self.connect_info()
            self.expect(self.srv.registered == regs + 1 and info.get("refused") is False, "404 with no controllers: a new id, open", odc.dev_text(new_dev))
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

        # A remote brightness, then a planned restart after the 5 s save
        # window: it is kept (NVS and the card's copy, which wins at boot).
        rec = self.rec()
        f = odc.sign_command(key, odc.command_body(dev, bytes.fromhex(rec.sess), 10**13, "brightness", {"v": 77}))
        got = await self.injected_ack("a brightness before the restart", f, tk)
        self.expect(got.get("ok"), "remote brightness 77 before a restart", json.dumps(got))
        await asyncio.sleep(8)
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
        cfg = (await self.http("GET", "/api/config"))[1] or {}
        self.expect(cfg.get("brightness") == 77, "a remote brightness survives a restart after the save window", f"brightness {cfg.get('brightness')}")
        st = await self.status()
        self.expect(not self.emu.failure and self.emu.resets <= self.emu.resets_allowed, "no panic, no unplanned restart", f"resets {self.emu.resets}")

    # --- pairing ------------------------------------------------------------------
    async def open_window(self, qr=False, key=None):
        """POST /api/connect/pair: (secret, the answer)."""
        code, r = await self.http("POST", "/api/connect/pair" + ("?qr=1" if qr else ""), key=key)
        if code != 200 or not r or not str(r.get("link", "")).startswith("https://opendmd.dev/connect/#pair="):
            raise Fail(f"POST /api/connect/pair: {code} {r}")
        return r["link"].split("#pair=", 1)[1], r

    async def guest(self, http, secret, ctl, macs=(None,), raw_label=None, timeout=60):
        """A new browser pairing as a guest: find, guest, one `pair` a MAC
        (None: the right one) and its `paired`."""
        pid = odc.pair_id(secret)
        async with http.post(f"{self.ctl_base}/v1/find", data=odc.dumps({"pair_id": pid.hex()})) as r:
            if r.status != 200:
                return {"find": r.status}
            found = odc.parse_dev((await r.json())["dev"])
        ws = await ctl.connect(http)
        await ctl.until("challenge", timeout=30)
        await ws.send_str(odc.dumps({"t": "guest", "pair_id": pid.hex()}))
        if await ctl.until("guest-ok", timeout=30) is None:
            return {"closed": await ctl.closed()}
        label = ctl.label if raw_label is None else raw_label
        replies = []
        for m in macs:
            m = m or odc.pair_mac(secret, found, ctl.pub, label.encode())
            await ws.send_str(odc.dumps({"t": "pair", "pub": ctl.pub.hex(), "label": label, "mac": m.hex()}))
            replies.append(await ctl.until("paired", timeout=timeout))
        await ws.close()
        return {"dev": found, "replies": replies}

    async def inject_pair(self, secret, dev, ctl, g=99):
        """A `pair` with the right MAC put straight onto the panel's socket
        (past the server's window check): the panel's own answer."""
        start = len(self.srv.said)
        await self.panel_ws().send_str(odc.dumps({"t": "pair", "g": g, "pub": ctl.pub.hex(), "label": ctl.label, "mac": odc.pair_mac(secret, dev, ctl.pub, ctl.label.encode()).hex()}))
        return await self.panel_says("the panel's answer to a pair put on its socket", "paired", start, lambda m: m.get("g") == g, timeout=120)

    async def screen_says(self, what, mark, timeout=120):
        async def seen():
            hits = [m.group(1) for _, m in self.emu.find(SCREEN, mark)]
            return what in hits
        await self.wait(f"the panel's screen: {what}", seen, timeout, every=1)

    async def pairing(self, http, dev, a, tk):
        from roundtrip import Controller

        rec = self.rec()
        # Red: with an API key set, the pair route wants it like the rest.
        code, _ = await self.http("PUT", "/api/config", {"api_key": LANE_KEY})
        self.expect(code == 200, "an API key set", f"{code}")
        try:
            code, _ = await self.http("POST", "/api/connect/pair?qr=1")
            self.expect(code == 401 and (await self.http("GET", "/api/connect", key=LANE_KEY))[1].get("pairing") is None,
                        "red: POST /api/connect/pair without the API key: 401, no window", f"{code}")
            secret, r = await self.open_window(key=LANE_KEY)
            self.expect(r.get("expires_s") == PAIR_TTL_S, "POST /api/connect/pair with the API key: a link", f"expires_s {r.get('expires_s')}")
            code, _ = await self.http("DELETE", "/api/connect/pair")
            self.expect(code == 401, "red: DELETE /api/connect/pair without the API key: 401", f"{code}")
            code, _ = await self.http("DELETE", "/api/connect/pair", key=LANE_KEY)
            self.expect(code == 200, "DELETE /api/connect/pair with the key", f"{code}")
        finally:
            await self.http("PUT", "/api/config", {"api_key": ""}, key=LANE_KEY)
        await asyncio.sleep(2)

        # Blue: the LAN link with its QR code on the panel, paired.
        mark, line = len(self.srv.said), self.emu.mark()
        secret, r = await self.open_window(qr=True)
        self.expect(len(secret) == 20 and r.get("qr") is True and r.get("online") is True, "POST /api/connect/pair?qr=1: a link, online", f"{r.get('link', '')[:40]}...")
        pid = odc.pair_id(secret).hex()
        await self.panel_says("pair-open for the window", "pair-open", mark, lambda m: m.get("pair_id") == pid)
        await self.screen_says("pairing QR code", line)
        info = await self.connect_info()
        self.expect((info.get("pairing") or {}).get("qr") is True and secret not in json.dumps(info), "GET /api/connect: the window, its QR, never its secret", json.dumps(info.get("pairing")))
        g1 = Controller(self.ctl_base, dev, "Den phone")
        mark, line = len(self.srv.said), self.emu.mark()
        res = await self.guest(http, secret, g1)
        rep = (res.get("replies") or [None])[0]
        self.expect(res.get("dev") == dev and rep and rep.get("k") == g1.k.hex(), "a guest found the panel by pair_id and paired", json.dumps(rep))
        self.expect(bytes.fromhex(rep.get("mac2", "")) == odc.paired_mac(secret, dev, g1.pub, g1.label.encode()), "paired's mac2: the secret, this panel, the key and the label as sent", "")
        said = self.srv.said[mark:]
        ki = next((i for i, m in enumerate(said) if m.get("t") == "keys" and any(e.get("k") == g1.k.hex() for e in m.get("keys", []))), None)
        pi = next((i for i, m in enumerate(said) if m.get("t") == "paired" and m.get("k") == g1.k.hex()), None)
        self.expect(ki is not None and pi is not None and ki < pi, "keys (with the new key) before paired", f"keys at {ki}, paired at {pi}")
        await self.panel_says("pair-close after the pairing", "pair-close", mark, lambda m: m.get("pair_id") == pid)
        self.ok("pair-close after the pairing")
        self.expect(rec.keys.get(g1.k, (None, None))[1] == odc.FLAG_CAN_ADD, "the server has the new key, can_add", "")
        await self.screen_says("added", line)
        self.ok("the panel says Remote control added")
        info = await self.connect_info()
        ent = next((c for c in info.get("controllers", []) if c.get("k") == g1.k.hex()), {})
        self.expect(ent.get("label") == "Den phone" and ent.get("f") == 1 and ent.get("added_by") is None and info.get("pairing") is None,
                    "GET /api/connect: the device, labelled, paired on the panel; no window", json.dumps(ent))
        at = await g1.attach(http)
        self.expect(at and at.get("online"), "the paired device attaches", "")
        r = await g1.command("controllers")
        if r is None:
            r = await g1.until("ack", timeout=60)
        self.expect(r and r.get("ok"), "the paired device's command: ok", json.dumps(r)[:200])
        await self.screen_says("off", line, timeout=180)
        self.ok("the added screen goes by itself; playback resumes")

        # Red: the window used once.
        res = await self.guest(http, secret, Controller(self.ctl_base, dev, "late"))
        self.expect(res.get("find") == 404, "red: a used window is gone at the server (find 404)", json.dumps(res))
        got = await self.inject_pair(secret, dev, Controller(self.ctl_base, dev, "late2"))
        self.expect(got.get("err") == "closed", "red: a used window's pair put on the panel's socket: closed", json.dumps(got))

        # Red: three wrong MACs close it, and the server hears at once.
        await asyncio.sleep(1.5)
        mark = len(self.srv.said)
        secret2, _ = await self.open_window()
        pid2 = odc.pair_id(secret2).hex()
        await self.panel_says("pair-open for the second window", "pair-open", mark, lambda m: m.get("pair_id") == pid2)
        x = Controller(self.ctl_base, dev, "x")
        mark = len(self.srv.said)
        res = await self.guest(http, secret2, x, macs=[bytes(32)] * 3 + [None])
        errs = [r.get("err") if r else None for r in res.get("replies", [])]
        self.expect(errs == ["bad-mac", "bad-mac", "bad-mac", "closed"], "red: three wrong MACs, then even the right one: closed", f"{errs}")
        said = self.srv.said[mark:]
        close_at = next((i for i, m in enumerate(said) if m.get("t") == "pair-close" and m.get("pair_id") == pid2), None)
        third = [i for i, m in enumerate(said) if m.get("t") == "paired" and m.get("err") == "bad-mac"]
        self.expect(close_at is not None and len(third) == 3 and close_at > third[2], "red: pair-close as soon as the third wrong MAC closed it", f"pair-close at {close_at}, bad-macs at {third}")
        self.expect(x.k not in rec.keys and (await self.connect_info()).get("pairing") is None, "red: no key stored, no window", "")

        # Red: a label rewritten by the relay; the window stays for the right one.
        await asyncio.sleep(1.5)
        secret3, _ = await self.open_window()
        await asyncio.sleep(2)
        g2 = Controller(self.ctl_base, dev, "Kitchen tablet")
        self.srv.rewrite_label = "Admin"
        try:
            res = await self.guest(http, secret3, g2)
        finally:
            self.srv.rewrite_label = None
        errs = [r.get("err") if r else None for r in res.get("replies", [])]
        self.expect(errs == ["bad-mac"] and g2.k not in rec.keys, "red: a label rewritten on the way: bad-mac, nothing stored", f"{errs}")
        res = await self.guest(http, secret3, g2)
        rep = (res.get("replies") or [None])[0]
        self.expect(rep and rep.get("k") == g2.k.hex(), "...and the window, two tries left, pairs the untouched one", json.dumps(rep))

        # A window cancelled on the LAN: pair-close, the QR code down.
        await asyncio.sleep(1.5)
        mark, line = len(self.srv.said), self.emu.mark()
        secret4, _ = await self.open_window(qr=True)
        pid4 = odc.pair_id(secret4).hex()
        await self.panel_says("pair-open (to cancel)", "pair-open", mark, lambda m: m.get("pair_id") == pid4)
        await self.screen_says("pairing QR code", line)
        code, info = await self.http("DELETE", "/api/connect/pair")
        self.expect(code == 200 and info.get("pairing") is None, "DELETE /api/connect/pair: no window", f"{code}")
        await self.panel_says("pair-close after a LAN cancel", "pair-close", mark, lambda m: m.get("pair_id") == pid4)
        await self.screen_says("off", line)
        self.ok("cancelled on the LAN: pair-close, the QR code down")

        # Red: a window left to expire.
        await asyncio.sleep(1.5)
        mark, line = len(self.srv.said), self.emu.mark()
        secret5, _ = await self.open_window(qr=True)
        pid5 = odc.pair_id(secret5).hex()
        await self.panel_says("pair-open (to expire)", "pair-open", mark, lambda m: m.get("pair_id") == pid5 and m.get("ttl", 0) <= PAIR_TTL_S)
        await self.screen_says("pairing QR code", line)
        await self.panel_says("pair-close at expiry", "pair-close", mark, lambda m: m.get("pair_id") == pid5, timeout=PAIR_TTL_S * 6 + 120)
        await self.screen_says("off", line)
        self.expect((await self.connect_info()).get("pairing") is None, "red: expired: pair-close by itself, the QR code down, no window", "")
        late = Controller(self.ctl_base, dev, "late3")
        got = await self.inject_pair(secret5, dev, late)
        self.expect(got.get("err") in ("expired", "closed") and late.k not in rec.keys, "red: a pair after expiry, put on the panel's socket: refused", json.dumps(got))

        su = ((await self.status()).get("connect") or {}).get("stack_unused")
        self.expect(isinstance(su, int) and su >= 1024, "the network worker's stack after pairing (MACs and key stores at the top level): >= 1 KB left", f"{su} B")
        self.report["stack_unused_after_pairing"] = su
        # The paired devices go again (the next steps count the list).
        await g1.ws.close()
        for k in (g1.k, g2.k):
            code, _ = await self.http("DELETE", f"/api/connect/controllers/{k.hex()}")
            self.expect(code == 200, f"the paired device {k.hex()[:6]} removed on the LAN", f"{code}")
        await asyncio.sleep(2)

    async def _moved(self, o0):
        return ((await self.status()).get("now_playing") or {}).get("ordinal") != o0

    async def _playing(self, path):
        return ((await self.status()).get("now_playing") or {}).get("path") == path

    async def _manifest(self, name):
        return name in ((await self.status()).get("manifest") or "")

    async def _new_record(self):
        return bool(self.srv.panels) and self.rec() and self.rec().panel is not None

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
    ap.add_argument("--seconds", type=int, default=7500, help="emulated seconds at most")
    ap.add_argument("--wall", type=int, default=12600, help="wall-clock limit (s)")
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
