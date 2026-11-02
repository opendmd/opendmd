# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Does CPU0's work reach the display? A raced Formula idle, then under load.

    uv run tools/display-stress.py HOST [--formula starfield] [--seconds 60]
        [--status-hz 20] [--config-every 2] [--upload-kb 256]

Plays a bundled Formula with `/api/play` (`formula_keep` set to `hold` for
the run, put back after), lets it settle, then samples
`/api/status` `scan.beam` (raced frames, late frames, late pairs, writes)
and `now_playing.formula` (frame µs, frame ms) over two phases of
`--seconds` each:

1. **idle**: one status read at the start and one at the end;
2. **stress**, at once: `/api/status` polled `--status-hz` times a second;
   a `PUT /api/config` every `--config-every` s that changes a setting
   (`dwell_max`, toggled between two values: a real NVS write when it is
   written, nothing the held Formula reacts to); and a file of `--upload-kb`
   KB written to the card and deleted, over and over
   (`/opendmd-stress.bin`).

The owner's rule: work on CPU0 must not reach the display. Pass when the
stress phase's late frames per 1,000 raced frames, its frame ms and its
frame µs (within a few %) match the idle phase's. Prints both phases and
the verdict; `--json OUT` keeps the numbers. Puts `dwell_max` and
`formula_keep` back and deletes the file at the end, whatever happens.
"""
import argparse
import json
import threading
import time
import urllib.request

STRESS_FILE = "/opendmd-stress.bin"


class Device:
    def __init__(self, host):
        self.base = f"http://{host}"

    def req(self, method, path, body=None, raw=None, timeout=15):
        data = raw if raw is not None else (json.dumps(body).encode() if body is not None else None)
        headers = {"Content-Type": "application/json"} if body is not None else {}
        r = urllib.request.Request(self.base + path, data=data, method=method, headers=headers)
        with urllib.request.urlopen(r, timeout=timeout) as f:
            return f.status, f.read().decode(errors="replace")

    def status(self):
        return json.loads(self.req("GET", "/api/status")[1])

    def config(self):
        return json.loads(self.req("GET", "/api/config")[1])


def snapshot(s):
    beam = ((s.get("scan") or {}).get("beam")) or {}
    f = (s.get("now_playing") or {}).get("formula") or {}
    return {
        "frames": beam.get("frames", 0),
        "late_frames": beam.get("late_frames", 0),
        "late_pairs": beam.get("late_pairs", 0),
        "late_starts": beam.get("late_starts", 0),
        "write_us": beam.get("write_us", 0),
        "frame_us": f.get("frame_us"),
        "frame_ms": f.get("frame_ms"),
        "misses": s.get("deadline_misses"),
        "heap": (s.get("heap") or {}).get("free"),
    }


def phase(a, b, seconds, extra=None):
    n = b["frames"] - a["frames"]
    late = b["late_frames"] - a["late_frames"]
    return {
        "seconds": seconds,
        "raced_frames": n,
        "late_frames": late,
        "late_per_1000": round(late * 1000 / n, 2) if n > 0 else None,
        "late_pairs": b["late_pairs"] - a["late_pairs"],
        "late_starts": b["late_starts"] - a["late_starts"],
        "frame_us": b["frame_us"],
        "frame_ms": b["frame_ms"],
        "misses": None if a["misses"] is None or b["misses"] is None else b["misses"] - a["misses"],
        **(extra or {}),
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("host")
    ap.add_argument("--formula", default="starfield")
    ap.add_argument("--seconds", type=float, default=60)
    ap.add_argument("--settle", type=float, default=8)
    ap.add_argument("--status-hz", type=float, default=20)
    ap.add_argument("--config-every", type=float, default=2)
    ap.add_argument("--upload-kb", type=int, default=256)
    ap.add_argument("--json")
    a = ap.parse_args()
    dev = Device(a.host)
    cfg = dev.config()
    keep, dwell_max = cfg.get("formula_keep", "turn"), int(cfg.get("dwell_max", 30))
    counts = {"status": 0, "config": 0, "uploads": 0, "errors": 0}
    stop = threading.Event()
    try:
        dev.req("PUT", "/api/config", {"formula_keep": "hold"})
        dev.req("POST", "/api/play", {"path": f"@formula:{a.formula}"})
        time.sleep(a.settle)
        s0 = snapshot(dev.status())
        time.sleep(a.seconds)
        s1 = snapshot(dev.status())
        idle = phase(s0, s1, a.seconds)

        def poll():
            gap = 1 / a.status_hz
            while not stop.is_set():
                try:
                    dev.status()
                    counts["status"] += 1
                except Exception:
                    counts["errors"] += 1
                time.sleep(gap)

        def configs():
            values = [dwell_max, dwell_max + 1]
            k = 0
            while not stop.is_set():
                k += 1
                try:
                    dev.req("PUT", "/api/config", {"dwell_max": values[k % 2]})
                    counts["config"] += 1
                except Exception:
                    counts["errors"] += 1
                stop.wait(a.config_every)

        def uploads():
            blob = bytes((i * 7) & 0xFF for i in range(a.upload_kb * 1024))
            while not stop.is_set():
                try:
                    dev.req("PUT", "/api/files" + STRESS_FILE, raw=blob, timeout=60)
                    dev.req("DELETE", "/api/files" + STRESS_FILE)
                    counts["uploads"] += 1
                except Exception:
                    counts["errors"] += 1
                    stop.wait(1)

        threads = [threading.Thread(target=f, daemon=True) for f in (poll, configs, uploads)]
        s2 = snapshot(dev.status())
        for t in threads:
            t.start()
        time.sleep(a.seconds)
        stop.set()
        for t in threads:
            t.join(timeout=70)
        s3 = snapshot(dev.status())
        stress = phase(s2, s3, a.seconds, dict(counts))
    finally:
        stop.set()
        for call in (
            lambda: dev.req("PUT", "/api/config", {"dwell_max": dwell_max, "formula_keep": keep}),
            lambda: dev.req("DELETE", "/api/files" + STRESS_FILE),
        ):
            try:
                call()
            except Exception:
                pass
    print(f"{a.formula}: idle   {idle}")
    print(f"{a.formula}: stress {stress}")
    li, ls = idle["late_per_1000"] or 0, stress["late_per_1000"] or 0
    us_i, us_s = idle["frame_us"] or 0, stress["frame_us"] or 0
    ok = (
        ls <= li + 0.5
        and stress["frame_ms"] == idle["frame_ms"]
        and (us_i == 0 or abs(us_s - us_i) <= us_i * 0.05)
    )
    print("verdict:", "PASS: the load did not reach the display" if ok else "FAIL: the load reached the display")
    if a.json:
        with open(a.json, "w") as f:
            json.dump({"formula": a.formula, "idle": idle, "stress": stress, "pass": ok}, f, indent=1)


if __name__ == "__main__":
    main()
