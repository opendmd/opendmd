# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Hammer the player's HTTP server while the panel is playing.

This is the display-timing stress test for the networking phase, not a
benchmark: the number that matters is what `/api/status` says about the frame
deadline afterwards, not the throughput.

    uv run tools/api-load.py <host> <seconds> [--workers 3] [--dir /gifs/Consoles]
    uv run tools/api-load.py <host> <seconds> [--workers 3] --paths paths.txt

Picks random GIFs out of `GET /api/files?dir=...` and fetches them concurrently
alongside a steady poll of `/api/status`, then prints throughput, error counts,
and the player's own metrics before and after — deadline misses, worst
lateness, heap floor.

`--paths FILE` swaps the GIF picking for a fixed list: a text file of
card-absolute paths (one per line, blank lines and `#` comments ignored,
e.g. `/cache/Consoles/3f/MD_Sonic_01.gif`) that the workers fetch directly
instead.
"""
import argparse
import http.client
import json
import random
import sys
import threading
import time
from urllib.parse import quote

class Stats:
    def __init__(self):
        self.lock = threading.Lock()
        self.requests = 0
        self.bytes = 0
        self.errors = 0
        self.statuses = {}

    def record(self, n_bytes, status):
        with self.lock:
            self.requests += 1
            self.bytes += n_bytes
            self.statuses[status] = self.statuses.get(status, 0) + 1
            if status != 200:
                self.errors += 1

    def fail(self, why):
        with self.lock:
            self.requests += 1
            self.errors += 1
            self.statuses[why] = self.statuses.get(why, 0) + 1

def get(host, path, timeout=30.0):
    conn = http.client.HTTPConnection(host, timeout=timeout)
    try:
        conn.request("GET", path)
        resp = conn.getresponse()
        body = resp.read()
        return resp.status, body
    finally:
        conn.close()

def get_json(host, path):
    status, body = get(host, path)
    if status != 200:
        raise RuntimeError(f"GET {path} -> HTTP {status}")
    return json.loads(body)

def snapshot(host):
    s = get_json(host, "/api/status")
    return {
        "deadline_misses": s["deadline_misses"],
        "worst_lateness_ms": s["worst_lateness_ms"],
        "heap_floor": s["heap"]["floor"],
        "heap_free": s["heap"]["free"],
        "gifs_started": s["gifs_started"],
        "refresh_hz": s["refresh_hz"],
        "uptime_secs": s["uptime_secs"],
    }

def print_snapshot(label, snap):
    print(f"{label:<10} misses={snap['deadline_misses']:<7} "
          f"worst={snap['worst_lateness_ms']:<5} ms  "
          f"floor={snap['heap_floor']:>7,} B  free={snap['heap_free']:>7,} B  "
          f"refresh={snap['refresh_hz']} Hz  gifs={snap['gifs_started']}  "
          f"up={snap['uptime_secs']}s")

def fetch_worker(host, paths, deadline, stats, stop, prefix="/gifs/"):
    while time.monotonic() < deadline and not stop.is_set():
        path = random.choice(paths)
        try:
            status, body = get(host, prefix + quote(path))
            stats.record(len(body), status)
        except Exception as e:
            stats.fail(type(e).__name__)

def status_worker(host, deadline, stats, stop):
    while time.monotonic() < deadline and not stop.is_set():
        try:
            status, body = get(host, "/api/status", timeout=10)
            stats.record(len(body), status)
        except Exception as e:
            stats.fail(type(e).__name__)
        time.sleep(0.25)

def load_paths_file(path):
    paths = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            paths.append(line)
    return paths

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("host")
    ap.add_argument("seconds", type=float)
    ap.add_argument("--dir", default="/gifs/Consoles", help="card directory to pull GIFs from")
    ap.add_argument("--workers", type=int, default=3, help="concurrent GIF fetchers")
    ap.add_argument("--paths", metavar="FILE",
                     help="text file of card-absolute paths (one per line, "
                          "blank lines and # comments ignored) to fetch "
                          "instead of picking GIFs from --dir")
    args = ap.parse_args()

    if args.paths:
        print(f"loading paths from {args.paths} …", flush=True)
        paths = load_paths_file(args.paths)
        print(f"  {len(paths)} paths")
        if not paths:
            sys.exit(f"no paths in {args.paths}")
        prefix = ""
    else:
        print(f"listing {args.dir} …", flush=True)
        t0 = time.monotonic()
        listing = get_json(args.host, "/api/files?dir=" + quote(args.dir))
        names = [e["name"] for e in listing["entries"]
                 if not e["dir"] and e["name"].lower().endswith(".gif")]
        print(f"  {len(names)} GIFs in {time.monotonic() - t0:.1f} s")
        if not names:
            sys.exit(f"no GIFs under {args.dir}")
        # `/gifs/<dir tail>/<name>`: the route is rooted at the card's gifs/.
        tail = args.dir.strip("/")
        tail = tail[len("gifs/"):] if tail.startswith("gifs/") else tail
        paths = [f"{tail}/{n}" if tail else n for n in names]
        prefix = "/gifs/"

    before = snapshot(args.host)
    print_snapshot("before", before)

    stats = Stats()
    stop = threading.Event()
    deadline = time.monotonic() + args.seconds
    threads = [threading.Thread(target=fetch_worker, args=(args.host, paths, deadline, stats, stop, prefix))
               for _ in range(args.workers)]
    threads.append(threading.Thread(target=status_worker,
                                    args=(args.host, deadline, stats, stop)))
    started = time.monotonic()
    for t in threads:
        t.start()
    try:
        while any(t.is_alive() for t in threads):
            time.sleep(5)
            with stats.lock:
                mb = stats.bytes / 1e6
            print(f"  … {time.monotonic() - started:5.0f} s  {stats.requests:5d} req  "
                  f"{mb:7.1f} MB  {stats.errors} errors", flush=True)
    except KeyboardInterrupt:
        stop.set()
    for t in threads:
        t.join()
    elapsed = time.monotonic() - started

    after = snapshot(args.host)
    print()
    print(f"{stats.requests} requests, {stats.bytes:,} B in {elapsed:.1f} s "
          f"= {stats.bytes / elapsed / 1024:.1f} KB/s, {stats.errors} errors")
    print("statuses:", stats.statuses)
    print_snapshot("before", before)
    print_snapshot("after", after)
    print(f"delta      misses +{after['deadline_misses'] - before['deadline_misses']}, "
          f"worst {before['worst_lateness_ms']} -> {after['worst_lateness_ms']} ms, "
          f"floor {before['heap_floor']:,} -> {after['heap_floor']:,} B")
    if after["uptime_secs"] < before["uptime_secs"]:
        print("!! the device rebooted during the run")
        sys.exit(1)

if __name__ == "__main__":
    main()
