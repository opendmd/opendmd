# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""M4: 200 forced FULL handshakes 10 s apart (tickets off), during GIF play
and during the raced Formula. Gates (CONNECT.md 10): heap floor >= 26 KB;
largest block at the floor >= 8 KB; no added late frames; transitions_cut
not raised; each handshake <= 4 s. Reports the probe's (the network
worker's) stack high-water and net.heap_low at "connect".

    uv run tools/connect-s0/m4.py 192.168.0.178 --baseline .../m2-.../summary.json \\
        --play formula=@formula:<the same one>
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import s0lib  # noqa: E402
from handshake_job import handshake_gates, run_job  # noqa: E402

ap = s0lib.parser(__doc__)
ap.add_argument("--phases", default="gif,formula")
ap.add_argument("--play", action="append", default=[], metavar="PHASE=PATH")
ap.add_argument("--interactive", action="store_true")
ap.add_argument("--settle-s", type=float, default=20.0)
ap.add_argument("--n", type=int, default=200)
ap.add_argument("--interval-s", type=float, default=10.0)
ap.add_argument("--floor-kb", type=float, default=26)
ap.add_argument("--largest-kb", type=float, default=8)
ap.add_argument("--max-ms", type=int, default=4000)
s0lib.add_baseline(ap)
args = ap.parse_args()
base = s0lib.load_baseline(args.baseline)
run = s0lib.Run("m4", args)
s0lib.need_probe(run.sample("start"))
s0lib.wait_open(run)
for phase in args.phases.split(","):
    s0lib.start_phase(run, phase, args)
    before, after = run_job(run, args, phase, False, args.n, args.interval_s, args.n * args.interval_s * 1.5 + 120)
    run.phases[phase] = stats = run.phase_stats(phase)
    stats["connect_before"], stats["connect_after"] = before, after
    handshake_gates(run, phase, before, after, False, args.n, args.max_ms)
    run.gate_floor(phase, stats, args.floor_kb)
    la = stats.get("largest_at_floor")
    run.gate(f"{phase}: largest block at the floor >= {args.largest_kb:g} KB", None if la is None else la >= args.largest_kb * 1024, la, f">= {int(args.largest_kb * 1024)} B")
    run.gate_no_added_late(phase, stats, base, args.tolerance)
    for k in ["heap.transitions_cut", "sd.readahead.no_heap"]:
        rate = s0lib.get(base or {}, f"phases.{phase}.rates_per_h")
        got = stats.get("deltas", {}).get(k)
        if not rate or k not in rate or got is None:
            run.gate(f"{phase}: {k} not raised", None, got, "a baseline")
            continue
        allowed = int(rate[k] * stats["duration_s"] / 3600.0 + 0.999) + args.tolerance
        run.gate(f"{phase}: {k} not raised", got <= allowed, f"{got} (baseline predicts {rate[k] * stats['duration_s'] / 3600.0:.2f})", f"<= {allowed}")
    print(f"  stack never used (network worker): {after.get('stack_unused')} B; net.heap_low: {s0lib.get(run.rows[-1], 'net.heap_low.free')} at {run.rows[-1].get('net.heap_low.at')}", flush=True)
    stats["stack_unused"] = after.get("stack_unused")
    run.gate(f"{phase}: network worker stack >= 1 KB left", (after.get("stack_unused") or 0) >= 1024, after.get("stack_unused"), ">= 1024 B")
run.gate_no_restart()
sys.exit(run.finish())
