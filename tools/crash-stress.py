# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Stress the paths around the /api/play crash until the device falls over.

    uv run tools/crash-stress.py <host> [--minutes 30] [--pi 192.168.1.112]
        [--dead 192.168.1.254] [--seed N] [--log stress.jsonl]
        [--tolerate N] [--idle N]

Several client threads at once, each with random short delays:

- **control** (the crash's sequence): `PUT /api/config` turning Follow
  RePlayOS on or off, with `follow_host` the dead address (no ARP answer:
  the poll's connect times out), the Pi (`--pi`: refused while RePlayOS is
  down, answering when it is up) — interleaved with `POST /api/play` of
  a small Formula (`@formula?src=..`), `@clock` and a GIF, `GET /api/status`, and now and then a
  `POST /api/follow/test`;
- **download**: `GET` of a large GIF (> 1 MB), read whole, or dropped part
  way (the detached request's close path);
- **upload**: a small `PUT /api/files/stress/…` (0.1-8 KB), sometimes cut
  off half way (the taken-over socket's abort path);
- **idle** (`--idle N`, default 3, 0 for none): up to N keep-alive
  connections opened and left idle for 1-12 s, then reused once. With the
  other three threads that is 6 of the server's 7 socket slots, as a
  browser's page load would hold. More than 7 at once makes httpd's LRU
  purge close one — sometimes a connection whose request it has not read
  yet, which the client sees as a connection closed without a response:
  expected with `--idle 5`, so give it `--tolerate`.

Every request's status and latency goes to `--log` (JSON lines). The run
stops at the end of `--minutes`, at the first connection failure (unless
`--tolerate N` allows N; a device that still answers `/api/status` with its
uptime going forwards is reported as such), or at a reboot (`uptime_secs`
going backwards, or `reset_reason` changing), and prints what every thread
was doing. The Follow settings found at the start are put back at the end
(`follow`, `follow_host`; the code is never touched), and the upload files
are deleted (and their directory).

Pass: "PASS" on the last line, no reboot, 0 connection failures; see the
summary for per-endpoint counts, p50/p99/max latency, and the device's own
heap floor and deadline misses before and after.
"""
import argparse
import http.client
import json
import os
import random
import socket
import sys
import threading
import time
import urllib.parse
from collections import defaultdict, deque

STOP = threading.Event()
LOCK = threading.Lock()
RESULTS = defaultdict(list)  # endpoint -> [latency_ms]
STATUSES = defaultdict(lambda: defaultdict(int))  # endpoint -> status -> n
CONN_FAILS = []
RECENT = defaultdict(lambda: deque(maxlen=8))  # thread -> recent actions
REASON = []
LOGF = None
T0 = time.monotonic()


class Dev:
    def __init__(self, host, timeout):
        self.host = host
        self.timeout = timeout


def record(thread, endpoint, status, ms, note=""):
    ev = {
        "t": round(time.monotonic() - T0, 3),
        "thread": thread,
        "ep": endpoint,
        "status": status,
        "ms": round(ms, 1),
    }
    if note:
        ev["note"] = note
    with LOCK:
        if isinstance(status, int):
            RESULTS[endpoint].append(ms)
        STATUSES[endpoint][status] += 1
        RECENT[thread].append(f"{ev['t']:8.2f}s {endpoint} -> {status} {ms:.0f} ms {note}")
        if LOGF:
            LOGF.write(json.dumps(ev) + "\n")


def request(dev, method, path, body=None, headers=None, conn=None, read=True):
    """One request; returns (status, body bytes, ms). Raises OSError/HTTPException."""
    own = conn is None
    c = conn or http.client.HTTPConnection(dev.host, 80, timeout=dev.timeout)
    t = time.monotonic()
    try:
        h = {"Connection": "close"} if own else {}
        h.update(headers or {})
        c.request(method, path, body=body, headers=h)
        r = c.getresponse()
        data = r.read() if read else b""
        return r.status, data, (time.monotonic() - t) * 1000, r
    finally:
        if own:
            c.close()


class Watch:
    """Reboot detection from every /api/status seen."""

    def __init__(self):
        self.uptime = -1
        self.reset = None
        self.first = None
        self.last = None

    def see(self, s):
        with LOCK:
            up = s.get("uptime_secs", -1)
            rr = s.get("reset_reason")
            if self.first is None:
                self.first = s
            self.last = s
            if up < self.uptime - 1:
                REASON.append(f"device rebooted: uptime {self.uptime} s -> {up} s, reset_reason {rr}")
                STOP.set()
            if self.reset is not None and rr != self.reset:
                REASON.append(f"reset_reason changed {self.reset} -> {rr} (uptime {up} s)")
                STOP.set()
            self.reset = rr
            self.uptime = max(self.uptime, up)


WATCH = Watch()


def status(dev, thread="control"):
    st, data, ms, _ = request(dev, "GET", "/api/status")
    record(thread, "GET /api/status", st, ms)
    if st == 200:
        WATCH.see(json.loads(data))
    return st


def conn_failure(dev, thread, what, exc, tolerate):
    """A request failed at the socket level: is the device still there?"""
    alive = None
    for _ in range(3):
        time.sleep(2)
        try:
            st, data, _, _ = request(dev, "GET", "/api/status")
            if st == 200:
                WATCH.see(json.loads(data))
                alive = json.loads(data).get("uptime_secs")
                break
        except (OSError, http.client.HTTPException):
            pass
    msg = f"[{thread}] {what}: {type(exc).__name__}: {exc}; device " + (
        f"answering, uptime {alive} s" if alive is not None else "NOT answering"
    )
    with LOCK:
        CONN_FAILS.append(msg)
        RECENT[thread].append(f"{time.monotonic() - T0:8.2f}s {what} -> {msg}")
        n = len(CONN_FAILS)
    print(msg, flush=True)
    if alive is None:
        REASON.append(f"device unreachable after: {msg}")
        STOP.set()
    elif n > tolerate:
        REASON.append(f"connection failure #{n} (tolerate {tolerate}): {msg}")
        STOP.set()


def pause(lo, hi):
    STOP.wait(random.uniform(lo, hi))

# A small Formula as the device writes its path: the source compacted, space
# as "+", and "% & + # ?" (and non-printable ASCII) as %XX.
FORMULA_SRC = "loop 4; gray(frac(x + t/4))"
FORMULA_PATH = "@formula?src=" + "".join(
    "+" if c == " " else f"%{ord(c):02X}" if c in "%&+#?" or not 32 < ord(c) < 127 else c for c in FORMULA_SRC)


def control(dev, args, gifs):
    hosts = [args.dead, args.pi]
    follow = True
    while not STOP.is_set():
        op = random.choices(
            ["config", "formula", "clock", "gif", "status", "test"],
            weights=[4, 3, 2, 2, 4, 1],
        )[0]
        try:
            if op == "config":
                follow = not follow if random.random() < 0.8 else follow
                host = random.choice(hosts)
                body = json.dumps({"follow": follow, "follow_host": host})
                st, _, ms, _ = request(dev, "PUT", "/api/config", body=body,
                                       headers={"Content-Type": "application/json"})
                record("control", "PUT /api/config", st, ms, f"follow={follow} host={host}")
                # The crash came 0.2 s after one of these: play right away.
                if random.random() < 0.6:
                    STOP.wait(random.uniform(0.05, 0.4))
                    op = random.choice(["formula", "gif", "clock"])
            if op in ("formula", "clock", "gif"):
                path = {"formula": FORMULA_PATH, "clock": "@clock"}.get(op) or random.choice(gifs)
                st, _, ms, _ = request(dev, "POST", "/api/play", body=json.dumps({"path": path}),
                                       headers={"Content-Type": "application/json"})
                record("control", "POST /api/play", st, ms, path)
            elif op == "status":
                status(dev)
            elif op == "test":
                body = json.dumps({"host": random.choice(hosts)})
                st, _, ms, _ = request(dev, "POST", "/api/follow/test", body=body,
                                       headers={"Content-Type": "application/json"})
                record("control", "POST /api/follow/test", st, ms)
        except (OSError, http.client.HTTPException) as e:
            conn_failure(dev, "control", op, e, args.tolerate)
        pause(0.05, 1.5)


def download(dev, args, big):
    while not STOP.is_set():
        path = random.choice(big)
        cut = random.random() < 0.3
        t = time.monotonic()
        try:
            c = http.client.HTTPConnection(dev.host, 80, timeout=dev.timeout)
            c.request("GET", path, headers={"Connection": "close"})
            r = c.getresponse()
            n = 0
            want = int(r.getheader("Content-Length") or 0)
            stop_at = random.randint(1, max(want // 2, 1)) if cut else None
            while True:
                chunk = r.read(8192)
                if not chunk:
                    break
                n += len(chunk)
                if stop_at and n >= stop_at:
                    break
            c.close()
            ms = (time.monotonic() - t) * 1000
            note = f"{n}/{want} B" + (" (dropped)" if cut else "")
            if r.status == 200 and not cut and n != want:
                note += " SHORT"
            record("download", "GET big", r.status, ms, note)
        except (OSError, http.client.HTTPException) as e:
            conn_failure(dev, "download", f"GET {path}", e, args.tolerate)
        pause(0.2, 3.0)


def upload(dev, args):
    k = 0
    while not STOP.is_set():
        k += 1
        path = f"/api/files/stress/up-{k % 3}.bin"
        size = random.randint(100, 8192)
        data = os.urandom(size)
        cut = random.random() < 0.2
        t = time.monotonic()
        try:
            if cut:
                s = socket.create_connection((dev.host, 80), timeout=dev.timeout)
                s.sendall(
                    f"PUT {path} HTTP/1.1\r\nHost: {dev.host}\r\nContent-Length: {size}\r\n"
                    f"Connection: close\r\n\r\n".encode() + data[: size // 2]
                )
                STOP.wait(random.uniform(0.0, 0.5))
                s.close()
                record("upload", "PUT cut", "cut", (time.monotonic() - t) * 1000, f"{size // 2}/{size} B")
            else:
                st, _, ms, _ = request(dev, "PUT", path, body=data,
                                       headers={"Content-Type": "application/octet-stream"})
                record("upload", "PUT /api/files", st, ms, f"{size} B")
        except (OSError, http.client.HTTPException) as e:
            conn_failure(dev, "upload", f"PUT {path}", e, args.tolerate)
        pause(0.3, 3.0)


def idle(dev, args):
    held = []
    while not STOP.is_set():
        try:
            if len(held) < args.idle and random.random() < 0.6:
                c = http.client.HTTPConnection(dev.host, 80, timeout=dev.timeout)
                st, _, ms, _ = request(dev, "GET", "/api/config", conn=c)
                record("idle", "GET /api/config (keep-alive)", st, ms)
                held.append((c, time.monotonic() + random.uniform(1, 12)))
            now = time.monotonic()
            for c, until in list(held):
                if now >= until:
                    # Reuse it once (maybe purged meanwhile: that is fine).
                    try:
                        st, _, ms, _ = request(dev, "GET", "/api/cache", conn=c)
                        record("idle", "GET /api/cache (reused)", st, ms)
                    except (OSError, http.client.HTTPException) as e:
                        record("idle", "GET /api/cache (reused)", "purged", 0, type(e).__name__)
                    c.close()
                    held.remove((c, until))
        except (OSError, http.client.HTTPException) as e:
            conn_failure(dev, "idle", "open keep-alive", e, args.tolerate)
        pause(0.2, 2.0)
    for c, _ in held:
        c.close()


def pct(v, p):
    v = sorted(v)
    return v[min(len(v) - 1, int(len(v) * p))] if v else 0


def main():
    global LOGF
    ap = argparse.ArgumentParser()
    ap.add_argument("host")
    ap.add_argument("--minutes", type=float, default=30)
    ap.add_argument("--pi", default="192.168.1.112")
    ap.add_argument("--dead", default="192.168.1.254")
    ap.add_argument("--seed", type=int, default=None)
    ap.add_argument("--log", default="crash-stress.jsonl")
    ap.add_argument("--tolerate", type=int, default=0)
    ap.add_argument("--idle", type=int, default=3)
    ap.add_argument("--timeout", type=float, default=30)
    args = ap.parse_args()
    seed = args.seed if args.seed is not None else random.randrange(1 << 30)
    random.seed(seed)
    dev = Dev(args.host, args.timeout)
    LOGF = open(args.log, "a", buffering=1)

    st, data, _, _ = request(dev, "GET", "/api/config")
    before = json.loads(data)
    keep = {"follow": before["follow"], "follow_host": before["follow_host"]}
    st, data, _, _ = request(dev, "GET", "/api/status")
    s0 = json.loads(data)
    WATCH.see(s0)
    # A listing with sizes stats every file (minutes in gifs/Consoles, and
    # it holds the card): list names only, then stat a sample for the big ones.
    st, data, _, _ = request(dev, "GET", "/api/files?dir=/gifs/Consoles")
    names = [e["name"] for e in json.loads(data)["entries"]
             if not e.get("dir") and e["name"].lower().endswith(".gif")]
    gifs = [f"/gifs/Consoles/{n}" for n in random.sample(names, min(20, len(names)))]
    big, fallback = [], []
    for n in random.sample(names, min(60, len(names))):
        path = f"/gifs/Consoles/{n}"
        st, data, _, _ = request(dev, "GET", f"/api/stat?path={urllib.parse.quote(path)}")
        size = (json.loads(data).get("size") or 0) if st == 200 else 0
        (big if size > 1_000_000 else fallback if size > 300_000 else []).append(path)
        if len(big) >= 6:
            break
    big = big or fallback[:6] or gifs[:3]
    print(f"seed {seed}; {len(gifs)} GIFs to play, {len(big)} big files; follow was {keep}; "
          f"uptime {s0['uptime_secs']} s, reset_reason {s0['reset_reason']}, version {s0['version']}",
          flush=True)

    threads = [
        threading.Thread(target=control, args=(dev, args, gifs), name="control"),
        threading.Thread(target=download, args=(dev, args, big), name="download"),
        threading.Thread(target=upload, args=(dev, args), name="upload"),
    ]
    if args.idle > 0:
        threads.append(threading.Thread(target=idle, args=(dev, args), name="idle"))
    for t in threads:
        t.daemon = True
        t.start()
    end = time.monotonic() + args.minutes * 60
    last = 0.0
    try:
        while not STOP.is_set() and time.monotonic() < end:
            STOP.wait(1)
            if time.monotonic() - last >= 60:
                last = time.monotonic()
                n = sum(sum(v.values()) for v in STATUSES.values())
                w = WATCH.last or {}
                print(f"{(time.monotonic() - T0) / 60:5.1f} min: {n} requests, "
                      f"{len(CONN_FAILS)} conn failures, uptime {w.get('uptime_secs')} s, "
                      f"heap {w.get('heap')}", flush=True)
    except KeyboardInterrupt:
        REASON.append("interrupted")
    STOP.set()
    for t in threads:
        t.join(timeout=args.timeout + 5)

    # Put Follow back and clear the scratch files (best effort).
    try:
        request(dev, "PUT", "/api/config", body=json.dumps(keep),
                headers={"Content-Type": "application/json"})
        for k in range(3):
            request(dev, "DELETE", f"/api/files/stress/up-{k}.bin")
        request(dev, "DELETE", "/api/files/stress")
        st, data, _, _ = request(dev, "GET", "/api/status")
        WATCH.see(json.loads(data))
    except (OSError, http.client.HTTPException) as e:
        print(f"cleanup failed: {e}")

    print(f"\n=== {(time.monotonic() - T0) / 60:.1f} min, seed {seed}")
    for ep in sorted(STATUSES):
        lat = RESULTS.get(ep, [])
        codes = ", ".join(f"{k}: {v}" for k, v in sorted(STATUSES[ep].items(), key=str))
        print(f"{ep:32s} {codes:40s} p50 {pct(lat, .5):6.0f} ms  p99 {pct(lat, .99):6.0f} ms  max {max(lat, default=0):6.0f} ms")
    a, b = WATCH.first or {}, WATCH.last or {}
    for key in ("uptime_secs", "reset_reason", "heap", "deadline_misses", "streams_aborted", "log_drops"):
        print(f"{key:18s} {a.get(key)!s:>40.40s} -> {b.get(key)!s:.60s}")
    print(f"follow restored to {keep}")
    for t in RECENT:
        print(f"-- last actions of {t}:")
        for line in RECENT[t]:
            print("   " + line)
    if CONN_FAILS:
        print("connection failures:")
        for m in CONN_FAILS:
            print("   " + m)
    if REASON:
        print("STOPPED: " + "; ".join(REASON))
        print("FAIL")
        sys.exit(1)
    print("PASS" if not CONN_FAILS else "PASS (with tolerated connection failures)")


if __name__ == "__main__":
    main()
