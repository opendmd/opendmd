# /// script
# requires-python = ">=3.11"
# ///
"""Summarise a probe build's serial capture between two device timestamps.

The firmware's timing probe (`firmware/src/probe.rs`, built with
`make fw-build FW_FEATURES=probe` or `probe,probe-http`) prints deferred
`sdopen` lines, `miss` lines attributing every frame-deadline miss, `http`
lines from the server (probe-http only) and an `sd:` part on every heartbeat.
This turns a capture of those (`tools/serial-capture.py`) into tables: open
latency per directory, misses by cause, the httpd operations that overlapped
them, and the blit/rewind figures from the heartbeats.

usage: uv run tools/sdprobe-report.py LOG [T0_MS [T1_MS]] [--boot N] [--label TEXT]

T0/T1 are the device's log timestamps (the `(12345)` in `I (12345) ...`);
omitted, the whole boot is used. Those restart at every reboot, so a capture
spanning several boots needs `--boot N`: only the Nth boot in the file
(1-based, counted by the firmware's banner line; -1 = the last) is read.
"""
import argparse
import re
import statistics
from collections import defaultdict

ap = argparse.ArgumentParser()
ap.add_argument("log")
ap.add_argument("t0", nargs="?", type=int, default=0)
ap.add_argument("t1", nargs="?", type=int, default=2**62)
ap.add_argument("--boot", type=int, default=0, help="only the Nth boot in the log (1-based, -1 = last)")
ap.add_argument("--label", default="")
a = ap.parse_args()

ansi = re.compile(r"\x1b\[[0-9;]*m")
line_re = re.compile(r"^\s*[\d.]*\s*([IWE]) \((\d+)\) (\S+): (.*)$")

opens = defaultdict(list)
misses = []
http = []
hb = []
starts = 0
bench = []
lines = [ansi.sub("", raw.decode("utf-8", "replace")).rstrip() for raw in open(a.log, "rb")]
if a.boot:
    banners = [i for i, l in enumerate(lines) if "OpenDMD firmware v" in l]
    k = a.boot - 1 if a.boot > 0 else len(banners) + a.boot
    lo = banners[k]
    hi = banners[k + 1] if k + 1 < len(banners) else len(lines)
    lines = lines[lo:hi]
for s in lines:
    m = line_re.match(s)
    if not m:
        continue
    ts = int(m.group(2))
    msg = m.group(4)
    if msg.startswith("blit bench"):
        bench.append(msg)
    if not (a.t0 <= ts <= a.t1):
        continue
    if msg.startswith("sdopen "):
        mm = re.match(r"sdopen (\d+) us (?:\(failed\) )?/sdcard(/.*)$", msg)
        us, path = int(mm.group(1)), mm.group(2)
        opens[path.rsplit("/", 1)[0]].append(us)
    elif msg.startswith("miss "):
        kv = {k: int(v) for k, v in re.findall(r"(\w+)[=~](\d+)", msg)}
        kv["ts"] = int(re.search(r"@(\d+)", msg).group(1))
        kv["path"] = msg.rsplit(" ", 1)[1]
        misses.append(kv)
    elif msg.startswith("http "):
        mm = re.match(r"http (\S+) (\d+) us (.*)$", msg)
        http.append((ts, mm.group(1), int(mm.group(2)), mm.group(3)))
    elif msg.startswith("heartbeat"):
        hb.append((ts, msg))
    elif re.match(r"\[\d+\] /", msg):
        starts += 1


def pct(xs, p):
    xs = sorted(xs)
    if not xs:
        return 0
    k = min(len(xs) - 1, max(0, round(p / 100 * (len(xs) - 1))))
    return xs[k]


t1 = a.t1 if a.t1 < 2**62 else (hb[-1][0] if hb else a.t0)
print(f"== {a.label}  window {a.t0}..{t1} ms ({(t1 - a.t0) / 60000:.1f} min), {starts} files started")
for b in bench:
    print(f"  {b}")

print("\n-- open latency by directory (ms) --")
print(f"{'dir':40} {'n':>4} {'p50':>7} {'p95':>7} {'max':>7}")
allv = []
for d, v in sorted(opens.items(), key=lambda kv: -statistics.median(kv[1])):
    allv += v
    print(f"{d:40} {len(v):4} {pct(v, 50) / 1000:7.1f} {pct(v, 95) / 1000:7.1f} {max(v) / 1000:7.1f}")
print(f"{'ALL':40} {len(allv):4} {pct(allv, 50) / 1000:7.1f} {pct(allv, 95) / 1000:7.1f} {max(allv or [0]) / 1000:7.1f}")
print(f"  opens {len(allv)} vs files started {starts}: {len(allv) - starts} not accounted for by a start (loop re-opens, retries)")


def attribute(m):
    """The biggest single cost in the missed iteration."""
    if m.get("opens", 0) > 0 and m["open"] >= m["late"] * 1000 * 0.5:
        return "file open (dir scan)"
    if m["rd_max"] >= 20000:
        return "slow read (>=20ms)"
    if m["log"] >= 5000:
        return "log/status (UART)"
    if m.get("cat", 0) >= 20000:
        return "catalog lookup"
    if m.get("blit", 0) >= m.get("cpu", 0):
        return "blit >= decode, sum > delay"
    return "decode > blit, sum > delay"


print(f"\n-- misses: {len(misses)}, worst {max([m['late'] for m in misses] or [0])} ms --")
by = defaultdict(list)
for m in misses:
    by[attribute(m)].append(m["late"])
for k, v in sorted(by.items(), key=lambda kv: -len(kv[1])):
    print(f"  {k:28} n={len(v):4}  late p50 {pct(v, 50):4} ms  max {max(v):5} ms")
if misses:
    delays = defaultdict(int)
    for m in misses:
        delays[m["delay"]] += 1
    print("  delay of missed frames:", dict(sorted(delays.items())))
    blit = [m["blit"] for m in misses]
    cpu = [m["cpu"] for m in misses]
    print(f"  blit p50 {pct(blit, 50) / 1000:.1f} ms, cpu(decode) p50 {pct(cpu, 50) / 1000:.1f} ms max {max(cpu) / 1000:.1f} ms")
    if any("px" in m for m in misses):
        px = [m.get("px", 0) for m in misses]
        print(f"  pixels pushed in missed frames: p50 {pct(px, 50)} max {max(px)}")
    rdm = [m["rd_max"] for m in misses]
    print(f"  rd_max in missed iterations: p50 {pct(rdm, 50) / 1000:.1f} ms max {max(rdm) / 1000:.1f} ms")
    reopen = sum(1 for m in misses if m.get("opens") and m.get("new") == 0)
    fresh = sum(1 for m in misses if m.get("opens") and m.get("new") == 1)
    print(f"  of which the open was a loop re-open (new=0): {reopen}, a new file: {fresh}")

if http:
    print(f"\n-- httpd events: {len(http)} --")
    kinds = defaultdict(list)
    for ts, k, us, d in http:
        kinds[k].append(us)
    for k, v in kinds.items():
        print(f"  {k:10} n={len(v):4} p50 {pct(v, 50) / 1000:8.1f} ms  p95 {pct(v, 95) / 1000:8.1f}  max {max(v) / 1000:8.1f} ms")
    # A miss whose iteration [ts - work, ts] overlaps an http event's span.
    spans = [(ts - us / 1000, ts, k, d) for ts, k, us, d in http]
    ov = defaultdict(int)
    ov_late = defaultdict(list)
    for m in misses:
        lo, hi = m["ts"] - m["work"] / 1000, m["ts"]
        hit = [k for (s, e, k, d) in spans if s <= hi + 10 and e >= lo - 10]
        key = f"{attribute(m)} | {'+'.join(sorted(set(hit))) or 'no httpd activity'}"
        ov[key] += 1
        ov_late[key].append(m["late"])
    print("  misses by (player cause | overlapping httpd op):")
    for k, n in sorted(ov.items(), key=lambda kv: -kv[1]):
        print(f"    {n:4}  worst {max(ov_late[k]):5} ms  {k}")

print("\n-- heartbeats --")
first = None
for ts, msg in hb:
    head = re.search(r"floor (\d+) B.*?(\d+) deadline misses \(worst (\d+) ms\)", msg)
    if head and first is None:
        first = head
    tail = msg.split("| sd:")[-1] if "| sd:" in msg else ""
    blit = re.search(r"blit p50 (\d+)us max (\d+)us \(run p50 (\d+)us max (\d+)us\) px/frame (\d+)", tail)
    rew = re.search(r"rewinds (\d+)", tail)
    opens_hb = re.search(r"opens (\d+) avg (\d+)us max (\d+)us", tail)
    parts = [f"@{ts}:"]
    if head:
        parts.append(f"floor {head.group(1)} misses {head.group(2)} worst {head.group(3)}")
    if blit:
        parts.append(f"blit p50 {blit.group(1)} max {blit.group(2)} us, px/frame {blit.group(5)}")
    if opens_hb:
        parts.append(f"opens {opens_hb.group(1)} max {int(opens_hb.group(3)) / 1000:.0f} ms")
    if rew:
        parts.append(f"rewinds {rew.group(1)}")
    print("  " + "  ".join(parts))
if hb:
    run = re.search(r"\(run p50 (\d+)us max (\d+)us\)", hb[-1][1])
    if run:
        print(f"  blit over the whole boot (last heartbeat): p50 {run.group(1)} us, max {run.group(2)} us")
if hb and first:
    last = re.search(r"floor (\d+) B.*?(\d+) deadline misses \(worst (\d+) ms\)", hb[-1][1])
    if last:
        print(
            f"  window delta: misses {int(last.group(2)) - int(first.group(2))} "
            f"(first heartbeat to last; counters are since boot), worst since boot {last.group(3)} ms"
        )
