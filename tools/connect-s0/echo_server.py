# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""A local stand-in for the echo Worker (connect/s0-echo), for the emulator
lane (m8_emu.py) only: TLS 1.2 with a test chain made here (`certs`), the
same four messages (ping, echo, stream, credit), `?k=` checked. Python's
standard library only; it listens where it is told (the lane: 10.0.2.2:8443
inside its own network namespace) and never connects anywhere.

    uv run tools/connect-s0/echo_server.py certs DIR          # a test root, intermediate and leaf
    uv run tools/connect-s0/echo_server.py serve DIR --listen 127.0.0.1:8443 --token T

The test hooks the lane uses (`Server` attributes): `mute_pings` (stop
answering pings, for the pong timeout), `close_all(code)` (the server
closes every session), `connections`, `handshakes` (full and resumed, from
OpenSSL's session_reused).
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import os
import socket
import ssl
import struct
import subprocess
import sys
import threading
from pathlib import Path

GUID = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
HOST = "connect-s0.test"


def payload_fill(st, n):
    x, k = st
    out = bytearray(n)
    for i in range(n):
        if k == 0:
            x ^= (x << 13) & 0xFFFFFFFF
            x ^= x >> 17
            x ^= (x << 5) & 0xFFFFFFFF
        out[i] = (x >> (8 * k)) & 0xFF
        k = (k + 1) & 3
    return (x, k), bytes(out)


def fnv(h, data):
    for b in data:
        h = ((h ^ b) * 0x01000193) & 0xFFFFFFFF
    return h


def make_certs(d: Path, name: str = HOST):
    """A P-384 root, a P-384 intermediate and a P-256 leaf for `name`, valid
    from yesterday for 30 days (like Cloudflare's shape: an ECDSA leaf under
    an ECDSA intermediate). Also an unrelated root and leaf (`other-*`), for
    the not-trusted case."""
    d.mkdir(parents=True, exist_ok=True)

    def run(*a):
        subprocess.run(["openssl", *a], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    for pre in ("", "other-"):
        run("ecparam", "-name", "secp384r1", "-genkey", "-noout", "-out", str(d / f"{pre}root.key"))
        run("req", "-x509", "-new", "-key", str(d / f"{pre}root.key"), "-sha384", "-days", "30", "-subj", f"/O=OpenDMD test/CN=S0 {pre}test root",
            "-addext", "basicConstraints=critical,CA:TRUE", "-addext", "keyUsage=critical,keyCertSign,cRLSign", "-out", str(d / f"{pre}root.pem"))
        run("ecparam", "-name", "secp384r1", "-genkey", "-noout", "-out", str(d / f"{pre}int.key"))
        run("req", "-new", "-key", str(d / f"{pre}int.key"), "-subj", f"/O=OpenDMD test/CN=S0 {pre}test intermediate", "-out", str(d / f"{pre}int.csr"))
        (d / "int.ext").write_text("basicConstraints=critical,CA:TRUE,pathlen:0\nkeyUsage=critical,keyCertSign,cRLSign\n")
        run("x509", "-req", "-in", str(d / f"{pre}int.csr"), "-CA", str(d / f"{pre}root.pem"), "-CAkey", str(d / f"{pre}root.key"), "-CAcreateserial",
            "-days", "29", "-sha384", "-extfile", str(d / "int.ext"), "-out", str(d / f"{pre}int.pem"))
        run("ecparam", "-name", "prime256v1", "-genkey", "-noout", "-out", str(d / f"{pre}leaf.key"))
        run("req", "-new", "-key", str(d / f"{pre}leaf.key"), "-subj", f"/CN={name}", "-out", str(d / f"{pre}leaf.csr"))
        (d / "leaf.ext").write_text(f"subjectAltName=DNS:{name}\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\n")
        run("x509", "-req", "-in", str(d / f"{pre}leaf.csr"), "-CA", str(d / f"{pre}int.pem"), "-CAkey", str(d / f"{pre}int.key"), "-CAcreateserial",
            "-days", "28", "-sha384", "-extfile", str(d / "leaf.ext"), "-out", str(d / f"{pre}leaf.pem"))
        (d / f"{pre}chain.pem").write_text((d / f"{pre}leaf.pem").read_text() + (d / f"{pre}int.pem").read_text())
    run("x509", "-in", str(d / "root.pem"), "-outform", "der", "-out", str(d / "root.der"))
    for f in d.glob("*.csr"):
        f.unlink()


class Server:
    def __init__(self, listen: str, certs: Path, token: str, chain: str = "chain.pem", key: str = "leaf.key"):
        host, port = listen.rsplit(":", 1)
        self.addr = (host, int(port))
        self.token = token
        self.ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        self.ctx.minimum_version = ssl.TLSVersion.TLSv1_2
        self.ctx.maximum_version = ssl.TLSVersion.TLSv1_2
        self.ctx.load_cert_chain(certs / chain, certs / key)
        self.ctx.set_ciphers("ECDHE-ECDSA-AES128-GCM-SHA256")
        self.lock = threading.Lock()
        self.socks: list = []
        self.mute_pings = False
        self.connections = 0
        self.upgrades = 0
        self.handshakes = {"full": 0, "resumed": 0}
        self.messages: list[str] = []
        self.refused = 0
        self.stopping = False
        self.close_gen = 0
        self.close_code = 1001

    def start(self):
        self.ls = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.ls.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.ls.bind(self.addr)
        self.ls.listen(8)
        threading.Thread(target=self._accept, daemon=True).start()
        return self

    def _accept(self):
        while not self.stopping:
            try:
                c, _ = self.ls.accept()
            except OSError:
                return
            threading.Thread(target=self._conn, args=(c,), daemon=True).start()

    def close_all(self, code=1001):
        """Every open session is closed by the server (its own thread sends
        the close frame and ends the connection: an SSL socket is not
        written from two threads)."""
        with self.lock:
            self.close_gen += 1
            self.close_code = code

    def _conn(self, raw):
        with self.lock:
            self.connections += 1
        try:
            s = self.ctx.wrap_socket(raw, server_side=True)
        except (ssl.SSLError, OSError):
            raw.close()
            return
        with self.lock:
            self.handshakes["resumed" if s.session_reused else "full"] += 1
        try:
            self._serve(s)
        except (OSError, ssl.SSLError, ConnectionError):
            pass
        finally:
            with self.lock:
                if s in self.socks:
                    self.socks.remove(s)
            try:
                s.close()
            except OSError:
                pass

    def _serve(self, s):
        head = b""
        while b"\r\n\r\n" not in head:
            d = s.recv(1024)
            if not d:
                return
            head += d
            if len(head) > 4096:
                return
        lines = head.split(b"\r\n")
        path = lines[0].split(b" ")[1].decode()
        hdr = {k.strip().lower(): v.strip() for k, v in (l.decode().split(":", 1) for l in lines[1:] if b":" in l)}
        from urllib.parse import parse_qs, urlparse
        u = urlparse(path)
        if u.path != "/ws" or parse_qs(u.query).get("k", [""])[0] != self.token:
            s.sendall(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 10\r\nConnection: close\r\n\r\nforbidden\n")
            with self.lock:
                self.refused += 1
            return
        acc = base64.b64encode(hashlib.sha1(hdr["sec-websocket-key"].encode() + GUID).digest()).decode()
        s.sendall(f"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {acc}\r\n\r\n".encode())
        with self.lock:
            self.upgrades += 1
            self.socks.append(s)
        stream = None
        buf = b""
        gen = self.close_gen
        s.settimeout(0.5)

        def closing():
            return self.close_gen != gen

        while True:
            fr, buf = read_frame(s, buf, closing)
            if fr == "close":
                s.sendall(frame(0x8, struct.pack(">H", self.close_code)))
                try:
                    s.unwrap()
                except (OSError, ssl.SSLError):
                    pass
                return
            if fr is None:
                return
            op, data = fr
            if op == 0x9:
                if not self.mute_pings:
                    s.sendall(frame(0xA, data))
            elif op == 0x8:
                s.sendall(frame(0x8, data[:2]))
                return
            elif op == 0x1:
                t = data.decode()
                with self.lock:
                    self.messages.append(t)
                w = t.split(" ")
                if t == "ping":
                    if not self.mute_pings:
                        s.sendall(frame(0x1, b"pong"))
                elif t.startswith("echo "):
                    if not self.mute_pings:
                        s.sendall(frame(0x1, data))
                elif w[0] == "stream" and len(w) == 5:
                    total, chunk, window, seed = map(int, w[1:])
                    stream = {"total": total, "left": total, "chunk": chunk, "st": (seed or 0x9E3779B9, 0), "h": 0x811C9DC5}
                    self._send(s, stream, window)
                elif w[0] == "credit" and len(w) == 2 and stream:
                    self._send(s, stream, int(w[1]))
                else:
                    s.sendall(frame(0x1, b"error unknown"))

    def _send(self, s, st, n):
        for _ in range(n):
            if st["left"] <= 0:
                break
            st["st"], out = payload_fill(st["st"], min(st["chunk"], st["left"]))
            st["h"] = fnv(st["h"], out)
            st["left"] -= len(out)
            s.sendall(frame(0x2, out))
        if st["left"] == 0:
            s.sendall(frame(0x1, f"end {st['total']} {st['h']:08x}".encode()))
            st["left"] = -1


def frame(op, data):
    n = len(data)
    if n < 126:
        h = bytes([0x80 | op, n])
    elif n < 65536:
        h = bytes([0x80 | op, 126]) + struct.pack(">H", n)
    else:
        h = bytes([0x80 | op, 127]) + struct.pack(">Q", n)
    return h + data


class Closing(Exception):
    pass


def read_frame(s, buf, closing=lambda: False):
    """One client frame: ((opcode, payload), rest), (None, buf) at the end,
    or ("close", buf) when `closing()` turns true while waiting."""
    def need(n):
        nonlocal buf
        while len(buf) < n:
            try:
                d = s.recv(4096)
            except (socket.timeout, TimeoutError):
                if closing():
                    raise Closing()
                continue
            if not d:
                raise ConnectionError("eof")
            buf += d

    try:
        need(2)
        op, b1 = buf[0] & 0x0F, buf[1]
        n, off = b1 & 0x7F, 2
        if n == 126:
            need(4)
            n, off = struct.unpack(">H", buf[2:4])[0], 4
        elif n == 127:
            need(10)
            n, off = struct.unpack(">Q", buf[2:10])[0], 10
        if not b1 & 0x80:
            raise ConnectionError("client frame not masked")
        need(off + 4 + n)
        mask = buf[off:off + 4]
        data = bytes(b ^ mask[i & 3] for i, b in enumerate(buf[off + 4:off + 4 + n]))
        return (op, data), buf[off + 4 + n:]
    except Closing:
        return "close", buf
    except (ConnectionError, OSError):
        return None, buf


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("certs")
    c.add_argument("dir", type=Path)
    s = sub.add_parser("serve")
    s.add_argument("dir", type=Path)
    s.add_argument("--listen", default="127.0.0.1:8443")
    s.add_argument("--token", default=os.environ.get("S0_TOKEN", ""))
    a = ap.parse_args()
    if a.cmd == "certs":
        make_certs(a.dir)
        print(f"certs in {a.dir}: root.der (the image's test root), chain.pem + leaf.key (the server's)")
        return
    Server(a.listen, a.dir, a.token).start()
    print(f"listening on {a.listen} (TLS 1.2, {HOST})", flush=True)
    threading.Event().wait()


if __name__ == "__main__":
    sys.exit(main())
