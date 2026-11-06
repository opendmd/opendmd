# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Does CPU0's work reach the display? A raced Formula idle, then under load.

    uv run tools/display-stress.py HOST [--formula starfield | --path PATH]
        [--jit on|off] [--seconds 60] [--loads poll,configs,uploads]
        [--status-hz 20] [--config-every 2] [--upload-kb 256] [--runs N]
        [--save-every 3] [--save-dir Consoles] [--json OUT]

`--runs N` repeats the idle and stress phases N times, playing the path
anew before each (where a Formula's memory lands varies with the heap):
each run keeps `display.buffers` (each buffer's SRAM pool), the heap's use
per pool idle and halfway through the stress phase (`/api/status?pools=1`,
and the difference: where the load's allocations sit), and the player's
longest card read and open during the stress phase.

`--loads saves` writes a new file into `/gifs/<--save-dir>` every
`--save-every` s and deletes it (a save's card work: two walks of a big
folder under the FAT lock); play a GIF with `--path` and the run is judged
by its deadline misses and the card read stalls.

The stress phase reports the upload KB/s and the status rate achieved
alongside the late frames.

`--path` plays any path instead of a bundled Formula, e.g. the bus test
(a frame of IRAM-resident VM code), with the JIT off for the run:

    P="@formula?psb=$(psi build --base64 tools/fixtures/iram-light.psi)"
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

The rule: work on CPU0 must not reach the display: no late frames
or missed refreshes. Pass when the stress phase's late frames per 1,000
raced frames are within the idle phase's + 0.5, its deadline misses within
idle's + 2 per 40 s, and its frame ms (the governor's step) the same. The
frame µs change (each phase's exact mean render) is reported, not judged:
a slowdown that only uses headroom passes. Prints both phases and
the verdict; `--json OUT` keeps the numbers. Puts `dwell_max` and
`formula_keep` back at the end, whatever happens (retried, and checked by
reading `/api/config` back; exits 2 with a loud message if they did not take),
and deletes the file.
"""
import argparse
import json
import sys
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


def restore(dev, want, tries=6):
    """Put the settings the run changed back and read /api/config to check
    they took; retried (the device may still be busy with the last upload).
    Says so loudly, and the script exits non-zero, if they did not."""
    last = None
    for k in range(tries):
        try:
            dev.req("PUT", "/api/config", want)
            got = dev.config()
            off = {key: got.get(key) for key, v in want.items() if got.get(key) != v}
            if not off:
                return True
            last = f"still {off}"
        except Exception as e:
            last = repr(e)
        time.sleep(1 + k)
    bar = "!" * 72
    print(f"{bar}\nRESTORE FAILED: the device keeps the run's settings ({last});\n"
          f"put these back by hand: PUT /api/config {json.dumps(want)}\n{bar}", file=sys.stderr)
    return False


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
        "render_us_total": f.get("render_us_total"),
        "renders": f.get("renders"),
        "frame_ms": f.get("frame_ms"),
        "misses": s.get("deadline_misses"),
        "heap": (s.get("heap") or {}).get("free"),
        "fat_index": ((s.get("sd") or {}).get("fat_index")) or {},
        "uploads": disp.get("uploads") or {},
        "miss_causes": ((s.get("misses") or {}).get("by_cause")) or {},
        "miss_late": ((s.get("misses") or {}).get("late_ms")) or {},
    }


def mean_render(a, b):
    """The mean render µs between snapshots `a` and `b`, or None (a firmware
    without the totals, or nothing rendered)."""
    if a.get("renders") is None or b.get("renders") is None:
        return None
    n = (b["renders"] - a["renders"]) % (1 << 32)
    us = (b["render_us_total"] - a["render_us_total"]) % (1 << 32)
    return round(us / n) if n else None


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
        # The phase's exact mean render (the totals' differences, wrapping
        # at 2^32 µs), and the governor's running mean at its end.
        "frame_us": mean_render(a, b) or b["frame_us"],
        "frame_us_end": b["frame_us"],
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
        # The player's card opens and reads under the writers' rule
        # (fatidx): refused under a write intent, raw reads that fell back,
        # opens that found every slot busy.
        # Uploads paced while a Formula ran near its budget (ms slept), and
        # whether pacing was on at the end.
        "upload_paced_ms": b["uploads"].get("paced_ms", 0) - a["uploads"].get("paced_ms", 0),
        # The phase's deadline misses by cause (open, read, decode, blit, …)
        # and by how late: what a GIF's late frames came from.
        "miss_causes": {k: v - a["miss_causes"].get(k, 0) for k, v in b["miss_causes"].items() if v - a["miss_causes"].get(k, 0)},
        "miss_late": {k: v - a["miss_late"].get(k, 0) for k, v in b["miss_late"].items() if v - a["miss_late"].get(k, 0)},
        "upload_paced_end": b["uploads"].get("paced"),
        "fat_index": {k: b["fat_index"].get(k, 0) - a["fat_index"].get(k, 0)
                      for k in ("intent_refused", "raw_fallbacks", "slots_full", "raw_reads") if k in b["fat_index"]},
        **(extra or {}),
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("host")
    ap.add_argument("--formula", default="starfield")
    ap.add_argument("--path", help="a path to play instead of @formula:<--formula> (a GIF for --loads saves)")
    ap.add_argument("--jit", choices=["on", "off"], help="formula_jit for the run (put back after)")
    ap.add_argument("--seconds", type=float, default=60)
    ap.add_argument("--settle", type=float, default=8)
    ap.add_argument("--runs", type=int, default=1, help="idle+stress pairs, the path played anew before each (where its memory lands varies)")
    ap.add_argument("--status-hz", type=float, default=20)
    ap.add_argument("--config-every", type=float, default=2)
    ap.add_argument("--upload-kb", type=int, default=256)
    ap.add_argument("--save-every", type=float, default=3, help="saves: s between new files")
    ap.add_argument("--save-dir", default="Consoles", help="saves: the /gifs/<dir> written to")
    ap.add_argument("--save-kb", type=int, default=24)
    ap.add_argument("--loads", default="poll,configs,uploads", help="which loads the stress phase runs, comma-separated (poll, configs, uploads, saves)")
    ap.add_argument("--json")
    a = ap.parse_args()
    dev = Device(a.host)
    cfg = dev.config()
    keep, dwell_max = cfg.get("formula_keep", "turn"), int(cfg.get("dwell_max", 30))
    jit_before = cfg.get("formula_jit")
    path = a.path or f"@formula:{a.formula}"
    label = a.formula if not a.path else "path"
    names = [n.strip() for n in a.loads.split(",") if n.strip()]
    runs = []
    restored = True
    stop = threading.Event()
    saved = []
    try:
        first = {"formula_keep": "hold"}
        if a.jit:
            first["formula_jit"] = a.jit == "on"
        dev.req("PUT", "/api/config", first)
        for k in range(a.runs):
            stop.clear()
            runs.append(one_run(dev, a, path, names, dwell_max, stop, saved))
            r = runs[-1]
            print(f"{label} run {k + 1}/{a.runs}: buffers {buffers_short(r['buffers'])}")
            print(f"{label} run {k + 1}: idle   {r['idle']}")
            print(f"{label} run {k + 1}: stress {r['stress']}")
            print(f"{label} run {k + 1}: verdict", "PASS" if r["pass"] else "FAIL")
    finally:
        stop.set()
        want = {"dwell_max": dwell_max, "formula_keep": keep} | (
            {"formula_jit": jit_before} if a.jit and jit_before is not None else {}
        )
        restored = restore(dev, want)
        for f in [STRESS_FILE] + saved:
            try:
                dev.req("DELETE", "/api/files" + f)
            except Exception:
                pass
    ok = all(r["pass"] for r in runs)
    print("verdict:", "PASS: the load did not reach the display" if ok else "FAIL: the load reached the display")
    if a.json:
        with open(a.json, "w") as f:
            out = {"formula": a.formula, "path": a.path, "jit": a.jit, "loads": a.loads, "runs": runs, "pass": ok}
            if len(runs) == 1:
                out |= {"idle": runs[0]["idle"], "stress": runs[0]["stress"]}
            json.dump(out, f, indent=1)
    if not restored:
        sys.exit(2)


def buffers_short(b):
    """`formula=SRAM2/15 ink=SRAM1/7 ...` from `display.buffers`."""
    if not b:
        return "(none reported)"
    out = []
    for k, v in b.items():
        if isinstance(v, dict):
            out.append(f"{k}={v.get('pool')}" + (f"..{v['pool_end']}" if v.get("pool_end") and v["pool_end"] != v.get("pool") else ""))
    return " ".join(out)


def one_run(dev, a, path, names, dwell_max, stop, saved):
    """One idle phase and one stress phase with the path played anew."""
    counts = {"status": 0, "config": 0, "uploads": 0, "saves": 0, "errors": 0}
    dev.req("POST", "/api/play", {"path": path})
    time.sleep(a.settle)
    st = dev.status()
    buffers = (st.get("display") or {}).get("buffers")
    s0 = snapshot(st)
    pools_idle = pools(dev)
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

    def saves():
        # A save's card work: a new file in a big folder (a directory walk
        # under the FAT lock to create it) and its delete (another).
        blob = bytes((i * 13) & 0xFF for i in range(a.save_kb * 1024))
        k = 0
        while not stop.is_set():
            k += 1
            f = f"/gifs/{a.save_dir}/zz-opendmd-stress-{k}.tmp"
            try:
                saved.append(f)
                dev.req("PUT", "/api/files" + f, raw=blob, timeout=60)
                dev.req("DELETE", "/api/files" + f)
                saved.remove(f)
                counts["saves"] += 1
            except Exception:
                counts["errors"] += 1
            stop.wait(a.save_every)

    loads = {"poll": poll, "configs": configs, "uploads": uploads, "saves": saves}
    for n in names:
        if n not in loads:
            raise SystemExit(f"--loads: unknown load {n!r} ({', '.join(loads)})")
    threads = [threading.Thread(target=loads[n], daemon=True) for n in names]
    take_read_max(dev)
    s2 = snapshot(dev.status())
    for t in threads:
        t.start()
    time.sleep(a.seconds / 2)
    pools_mid = pools(dev)
    time.sleep(a.seconds / 2)
    stop.set()
    for t in threads:
        t.join(timeout=70)
    s3 = snapshot(dev.status())
    read_max = take_read_max(dev)
    extra = dict(counts, loads=names, card=read_max)
    if "uploads" in names:
        # Throughput: what the upload loop moved (each a write and a delete).
        extra["upload_kb_s"] = round(counts["uploads"] * a.upload_kb / a.seconds, 1)
    if "poll" in names:
        extra["status_hz_asked"] = a.status_hz
        extra["status_hz_achieved"] = round(counts["status"] / a.seconds, 2)
    stress = phase(s2, s3, a.seconds, extra)
    li, ls = idle["late_per_1000"] or 0, stress["late_per_1000"] or 0
    us_i, us_s = idle["frame_us"] or 0, stress["frame_us"] or 0
    # The test: no late frames or missed refreshes
    # caused by non-display work. Late raced frames within idle + 0.5 per
    # 1,000, deadline misses no more than idle's, and no governor step-down
    # (the frame time it holds the Formula to unchanged). A slowdown that
    # only uses headroom passes: frame_us_change_pct is information.
    # (Deadline misses: within idle's + 2 per 40 s, so a single stray miss
    # is noise, not a failure.)
    ok = (
        ls <= li + 0.5
        and (stress["misses"] or 0) <= (idle["misses"] or 0) + max(1, round(2 * a.seconds / 40))
        and stress["frame_ms"] == idle["frame_ms"]
    )
    return {
        "buffers": buffers,
        "pools_idle": pools_idle,
        "pools_mid_stress": pools_mid,
        "pools_delta": pools_delta(pools_idle, pools_mid),
        "idle": idle,
        "stress": stress,
        "frame_us_change_pct": round((us_s / us_i - 1) * 100, 1) if us_i else None,
        "pass": ok,
    }


def pools(dev):
    """`/api/status?pools=1`'s pools, or None (a firmware without it)."""
    try:
        return json.loads(dev.req("GET", "/api/status?pools=1")[1]).get("pools")
    except Exception:
        return None


def pools_delta(a, b):
    """Bytes in use per pool, mid-stress less idle: where the load's
    allocations (network, upload, card) sit."""
    if not a or not b:
        return None
    return {k: b[k]["used"] - a.get(k, {}).get("used", 0) for k in b}


def take_read_max(dev):
    """The player's longest card read and open since the last call (reset)."""
    try:
        return json.loads(dev.req("GET", "/api/status?take_read_max=1")[1])
    except Exception:
        return None


if __name__ == "__main__":
    main()
