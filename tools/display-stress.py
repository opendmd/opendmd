# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Does CPU0's work reach the display? A raced Formula idle, then under load.

    uv run tools/display-stress.py HOST [--formula starfield | --path PATH]
        [--jit on|off] [--seconds 60] [--loads poll,configs,uploads]
        [--status-hz 20] [--config-every 2] [--upload-kb 256] [--json OUT]

`--quiet on|off` sets the quiet window (`PUT /api/quiet`) for the run, for
an A/B in one session; the stress phase reports the upload KB/s and the
status rate achieved alongside the late frames.

`--path` plays any path instead of a bundled Formula, e.g. the bus test
(a frame of IRAM-resident VM code), with the JIT off for the run:

    P="@formula?psb=$(psi build --base64 docs/research/display-iso/iram-light.psi)"
    uv run tools/display-stress.py HOST --path "$P" --jit off --loads uploads

Plays a bundled Formula with `/api/play` (`formula_keep` set to `hold` for
the run, put back after), lets it settle, then samples
`/api/status` `scan.beam` (raced frames, late frames, late pairs, writes)
and `now_playing.formula` (frame µs, frame ms) over two phases of
`--seconds` each:

1. **idle**: one status read at the start and one at the end;
2. **stress**, the loads `--loads` names at once (all three by default):
   `poll`, `/api/status` polled `--status-hz` times a second (the rate
   achieved is reported: a device that answers slowly gets fewer);
   `configs`, a `PUT /api/config` every `--config-every` s that changes a setting
   (`dwell_max`, toggled between two values: a real NVS write when it is
   written, nothing the held Formula reacts to); `uploads`, a file of
   `--upload-kb` KB written to the card and deleted, over and over
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
    disp = s.get("display") or {}
    pre = disp.get("pre_start") or {}
    cmd = disp.get("commands") or {}
    return {
        "pre_n": pre.get("n", 0),
        "pre_us": pre.get("us", 0),
        "pre_spikes": pre.get("spikes", 0),
        "cmd_n": cmd.get("n", 0),
        "cmd_us": cmd.get("us", 0),
        "cmd_max_us": cmd.get("max_us"),
        "status_cached": disp.get("status_cached", 0),
        "status_build_us": disp.get("status_build_us"),
        "status_lock_us": disp.get("status_lock_us"),
        "wakes": disp.get("wakes", 0),
        "wake_late": disp.get("wake_late", 0),
        "wake_max_us": disp.get("wake_max_us"),
        "publish_skipped": disp.get("publish_skipped", 0),
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
        "wakes": b["wakes"] - a["wakes"],
        "wake_late": b["wake_late"] - a["wake_late"],
        "wake_max_us_since_boot": b["wake_max_us"],
        "publish_skipped": b["publish_skipped"] - a["publish_skipped"],
        # The wake to a raced start: the mean over the phase and the
        # stretches (over the running mean by 300 µs).
        "pre_start_mean_us": round((b["pre_us"] - a["pre_us"]) / (b["pre_n"] - a["pre_n"])) if b["pre_n"] > a["pre_n"] else None,
        "pre_start_spikes": b["pre_spikes"] - a["pre_spikes"],
        "command_loops": b["cmd_n"] - a["cmd_n"],
        "command_us": b["cmd_us"] - a["cmd_us"],
        "command_max_us_since_boot": b["cmd_max_us"],
        "status_cached": b["status_cached"] - a["status_cached"],
        "status_build_us_last": b["status_build_us"],
        "status_lock_us_last": b["status_lock_us"],
        **(extra or {}),
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("host")
    ap.add_argument("--formula", default="starfield")
    ap.add_argument("--path", help="a path to play instead of @formula:<--formula>")
    ap.add_argument("--jit", choices=["on", "off"], help="formula_jit for the run (put back after)")
    ap.add_argument("--quiet", choices=["on", "off"], help="the quiet window (PUT /api/quiet) for the run (put back after)")
    ap.add_argument("--seconds", type=float, default=60)
    ap.add_argument("--settle", type=float, default=8)
    ap.add_argument("--status-hz", type=float, default=20)
    ap.add_argument("--config-every", type=float, default=2)
    ap.add_argument("--upload-kb", type=int, default=256)
    ap.add_argument("--loads", default="poll,configs,uploads", help="which loads the stress phase runs, comma-separated")
    ap.add_argument("--json")
    a = ap.parse_args()
    dev = Device(a.host)
    cfg = dev.config()
    keep, dwell_max = cfg.get("formula_keep", "turn"), int(cfg.get("dwell_max", 30))
    jit_before = cfg.get("formula_jit")
    quiet_before = None
    if a.quiet:
        try:
            quiet_before = json.loads(dev.req("GET", "/api/quiet")[1]).get("on")
            dev.req("PUT", "/api/quiet", {"on": a.quiet == "on"})
        except Exception as e:
            raise SystemExit(f"--quiet: this firmware has no /api/quiet ({e})")
    path = a.path or f"@formula:{a.formula}"
    label = a.formula if not a.path else "path"
    counts = {"status": 0, "config": 0, "uploads": 0, "errors": 0}
    stop = threading.Event()
    try:
        first = {"formula_keep": "hold"}
        if a.jit:
            first["formula_jit"] = a.jit == "on"
        dev.req("PUT", "/api/config", first)
        dev.req("POST", "/api/play", {"path": path})
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

        loads = {"poll": poll, "configs": configs, "uploads": uploads}
        names = [n.strip() for n in a.loads.split(",") if n.strip()]
        for n in names:
            if n not in loads:
                raise SystemExit(f"--loads: unknown load {n!r} (poll, configs, uploads)")
        threads = [threading.Thread(target=loads[n], daemon=True) for n in names]
        s2 = snapshot(dev.status())
        for t in threads:
            t.start()
        time.sleep(a.seconds)
        stop.set()
        for t in threads:
            t.join(timeout=70)
        s3 = snapshot(dev.status())
        extra = dict(counts, loads=names, quiet=a.quiet)
        if "uploads" in names:
            # Throughput: what the upload loop moved (each a write and a delete).
            extra["upload_kb_s"] = round(counts["uploads"] * a.upload_kb / a.seconds, 1)
        if "poll" in names:
            extra["status_hz_asked"] = a.status_hz
            extra["status_hz_achieved"] = round(counts["status"] / a.seconds, 2)
        stress = phase(s2, s3, a.seconds, extra)
    finally:
        stop.set()
        for call in (
            lambda: dev.req(
                "PUT",
                "/api/config",
                {"dwell_max": dwell_max, "formula_keep": keep}
                | ({"formula_jit": jit_before} if a.jit and jit_before is not None else {}),
            ),
            lambda: dev.req("DELETE", "/api/files" + STRESS_FILE),
            lambda: quiet_before is not None and dev.req("PUT", "/api/quiet", {"on": quiet_before}),
        ):
            try:
                call()
            except Exception:
                pass
    print(f"{label}: idle   {idle}")
    print(f"{label}: stress {stress}")
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
            json.dump({"formula": a.formula, "path": a.path, "jit": a.jit, "loads": a.loads, "idle": idle, "stress": stress, "pass": ok}, f, indent=1)


if __name__ == "__main__":
    main()
