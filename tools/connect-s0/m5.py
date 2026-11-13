# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""M5: 1,000 RESUMED reconnects (session tickets). Gates (CONNECT.md 10):
each <= 0.5 s; no drift: the free heap before an attempt and mbedTLS's
bytes held between sessions after 1,000 within 1 KB of after 10 (this
catches the reported 16 B-a-connection leak with dynamic buffers).

    uv run tools/connect-s0/m5.py 192.168.0.178 [--n 1000] [--interval-s 2]
"""
import json
import statistics
import sys
import time
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import s0lib  # noqa: E402
from handshake_job import handshake_gates, run_job  # noqa: E402

ap = s0lib.parser(__doc__)
ap.add_argument("--n", type=int, default=1000)
ap.add_argument("--interval-s", type=float, default=2.0)
ap.add_argument("--max-ms", type=int, default=500)
ap.add_argument("--drift-kb", type=float, default=1.0)
ap.add_argument("--settle-s", type=float, default=15.0, help="idle seconds before each heap reading")
ap.add_argument("--no-pause", action="store_true", help="read the heap with play going on (noisier: the free heap moves with each GIF)")
args = ap.parse_args()
run = s0lib.Run("m5", args)
s0lib.need_probe(run.sample("start"))
s0lib.reset_probe(run)
run.gate("the session opened", s0lib.wait_open(run), "open", "open within 180 s")


def set_paused(want: bool):
    """POST /api/pause toggles: until the panel says `want`."""
    for _ in range(3):
        req = urllib.request.Request(run.panel.base + "/api/pause", method="POST", data=b"")
        if args.key:
            req.add_header("Authorization", f"Bearer {args.key}")
        try:
            with urllib.request.urlopen(req, timeout=10) as r:
                if json.loads(r.read()).get("paused") is want:
                    return True
        except Exception as e:  # noqa: BLE001
            run.note(f"pause: {e}")
        time.sleep(1)
    return False


def idle_reading(label):
    """Medians over --settle-s of idle, the panel paused (fixed content)
    unless --no-pause: the free heap and mbedTLS's bytes."""
    paused = not args.no_pause and set_paused(True)
    if not args.no_pause and not paused:
        run.note(f"{label}: could not pause the panel; reading with play on")
    end = time.time() + args.settle_s
    free, tls = [], []
    while time.time() < end:
        st = run.sample(label)
        free.append(s0lib.get(st, "heap.free"))
        tls.append(s0lib.get(st, "connect.heap.tls_in_use"))
        time.sleep(max(1.0, args.sample_s / 2))
    if paused:
        set_paused(False)
    free = [f for f in free if isinstance(f, int)]
    tls = [t for t in tls if isinstance(t, int)]
    return (statistics.median(free) if free else None, statistics.median(tls) if tls else None)


# The first 10 (the first one makes the ticket), then the rest.
b0, a0 = run_job(run, args, "first-10", True, 10, args.interval_s, 10 * args.interval_s * 3 + 120)
# Both readings are of an open, idle session: else they compare nothing.
run.gate("open after the first 10", s0lib.wait_open(run, phase="first-10:open"), "open", "open")
free10, tls10 = idle_reading("after-10")
b1, a1 = run_job(run, args, "rest", True, args.n - 10, args.interval_s, (args.n - 10) * args.interval_s * 2 + 300)
run.gate(f"open after all {args.n}", s0lib.wait_open(run, phase="rest:open"), "open", "open")
free_n, tls_n = idle_reading(f"after-{args.n}")
run.phases["rest"] = run.phase_stats("rest")
handshake_gates(run, "all", b0, a1, True, args.n, args.max_ms)
# The gate: mbedTLS's own bytes held between sessions (exact, play cannot
# move it). The free heap is reported beside it (play moves it).
if tls10 is not None and tls_n is not None:
    run.gate(f"mbedTLS bytes held drift after {args.n} vs 10 within {args.drift_kb:g} KB", tls_n - tls10 <= args.drift_kb * 1024,
             f"{tls10} -> {tls_n} B ({tls_n - tls10:+} B, {(tls_n - tls10) / max(1, args.n - 10):.1f} B a connection)", f"growth <= {int(args.drift_kb * 1024)} B")
else:
    run.gate("mbedTLS bytes drift", None, "no readings", "")
run.gate(f"free heap drift after {args.n} vs 10 (reported)", None if free10 is None or free_n is None else free10 - free_n <= args.drift_kb * 1024,
         f"{free10} -> {free_n} B" + ("" if free10 is None or free_n is None else f" ({free_n - free10:+} B)"),
         f"loss <= {int(args.drift_kb * 1024)} B", required=False)
run.phases["drift"] = {"free_after_10": free10, "free_after_n": free_n, "tls_after_10": tls10, "tls_after_n": tls_n}
run.gate_no_restart()
sys.exit(run.finish())
